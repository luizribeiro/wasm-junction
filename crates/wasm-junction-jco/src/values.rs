use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use js_sys::{Array, BigInt, Object, Reflect, Uint8Array};
use wasm_bindgen::{JsCast, JsValue};
use wasm_junction_core::{
    CallError, ChannelDirection, EngineEvent, ImportDispatcher, InputStream, InvocationId,
    Resource, ResourceOwnership, StreamHandle, Val, Vals, validate_resource_lowering,
};

use crate::types::{FunctionType, ResourceType, ValueType};

const RESOURCE_MARKER: &str = "$wasm-junction-resource";
const STREAM_MARKER: &str = "$wasm-junction-stream";

#[derive(Clone, Default)]
pub(crate) struct ResourceTracker {
    resources: Rc<RefCell<HashSet<Resource>>>,
    host_streams: Rc<RefCell<HashMap<u64, InputStream>>>,
    guest_streams: Rc<RefCell<HashMap<u64, StreamHandle>>>,
    refuse_guest_streams: Rc<Cell<bool>>,
    imports: Option<Arc<dyn ImportDispatcher>>,
    invocation: Option<InvocationId>,
}

impl ResourceTracker {
    pub(crate) fn with_imports(
        imports: Arc<dyn ImportDispatcher>,
        invocation: Option<InvocationId>,
    ) -> Self {
        Self {
            imports: Some(imports),
            invocation,
            ..Self::default()
        }
    }

    fn retain(&self, resource: Resource) {
        self.resources.borrow_mut().insert(resource);
    }

    pub(crate) fn take(&self, interface: &str, name: &str, id: u32) -> Result<Resource, CallError> {
        let resource = Resource::owned(interface, name, id);
        self.resources.borrow_mut().take(&resource).ok_or_else(|| {
            CallError::refused(format!(
                "resource `{interface}/{name}#{id}` is no longer owned"
            ))
        })
    }

    pub(crate) fn drain(&self) -> Vec<Resource> {
        self.resources.borrow_mut().drain().collect()
    }

    pub(crate) fn register_guest(&self, handle: StreamHandle) -> JsValue {
        let id = handle.id();
        self.guest_streams.borrow_mut().insert(id, handle);
        self.channel_open(id, ChannelDirection::GuestToHost);
        stream_marker("guest", id)
    }

    pub(crate) fn take_host(&self, id: u64) -> Option<InputStream> {
        let input = self.host_streams.borrow_mut().remove(&id);
        if input.is_some() {
            self.channel_close(id, ChannelDirection::HostToGuest);
        }
        input
    }

    pub(crate) fn checkout_host(&self, id: u64) -> Option<InputStream> {
        self.host_streams.borrow_mut().remove(&id)
    }

    pub(crate) fn restore_host(&self, id: u64, input: InputStream) {
        self.host_streams.borrow_mut().insert(id, input);
    }

    pub(crate) fn finish_host(&self, id: u64) {
        self.channel_close(id, ChannelDirection::HostToGuest);
    }

    pub(crate) fn take_guest(&self, id: u64) -> Option<StreamHandle> {
        self.guest_streams.borrow_mut().remove(&id)
    }

    pub(crate) fn host_ids(&self) -> Vec<u64> {
        self.host_streams.borrow().keys().copied().collect()
    }

    pub(crate) fn close_guest(&self, id: u64) {
        self.channel_close(id, ChannelDirection::GuestToHost);
    }

    fn channel_open(&self, id: u64, direction: ChannelDirection) {
        if let (Some(imports), Some(invocation)) = (&self.imports, self.invocation) {
            imports.emit(EngineEvent::ChannelOpen {
                invocation,
                stream: id,
                direction,
            });
        }
    }

    fn channel_close(&self, id: u64, direction: ChannelDirection) {
        if let (Some(imports), Some(invocation)) = (&self.imports, self.invocation) {
            imports.emit(EngineEvent::ChannelClose {
                invocation,
                stream: id,
                direction,
            });
        }
    }
}

#[derive(Clone, Copy)]
enum ResourceRetention<'a> {
    Track(&'a ResourceTracker),
    Ignore,
}

pub(crate) enum JsResult {
    Return(JsValue),
    Throw(JsValue),
    Poison(JsValue),
}

#[cfg(test)]
pub(crate) fn lower_args(values: Vals, signature: &FunctionType) -> Result<Array, CallError> {
    lower_args_tracked(values, signature, &ResourceTracker::default())
}

pub(crate) fn lower_args_tracked(
    values: Vals,
    signature: &FunctionType,
    resources: &ResourceTracker,
) -> Result<Array, CallError> {
    if values.len() != signature.params.len() {
        return Err(CallError::trap("component argument count mismatch"));
    }
    values
        .into_iter()
        .zip(&signature.params)
        .map(|(value, expected)| lower(value, expected, ResourceRetention::Track(resources)))
        .collect()
}

#[allow(clippy::cast_possible_truncation)]
#[cfg(test)]
pub(crate) fn lift_args(values: &Array, signature: &FunctionType) -> Result<Vals, CallError> {
    lift_args_tracked(values, signature, &ResourceTracker::default())
}

pub(crate) fn lift_args_tracked(
    values: &Array,
    signature: &FunctionType,
    resources: &ResourceTracker,
) -> Result<Vals, CallError> {
    if values.length() as usize != signature.params.len() {
        return Err(CallError::trap("imported argument count mismatch"));
    }
    signature
        .params
        .iter()
        .enumerate()
        .map(|(index, ty)| lift(values.get(index as u32), ty, resources))
        .collect()
}

#[cfg(test)]
pub(crate) fn lower_result(values: &Vals, signature: &FunctionType) -> Result<JsResult, CallError> {
    lower_result_tracked(values, signature, &ResourceTracker::default())
}

pub(crate) fn default_result(signature: &FunctionType) -> Result<JsResult, CallError> {
    let Some(result) = &signature.result else {
        return Ok(JsResult::Poison(JsValue::UNDEFINED));
    };
    let result = match result {
        ValueType::Result { ok, .. } => match ok.as_deref() {
            Some(ok) => lower(default_value(ok)?, ok, ResourceRetention::Ignore)?,
            None => JsValue::UNDEFINED,
        },
        result => lower(default_value(result)?, result, ResourceRetention::Ignore)?,
    };
    Ok(JsResult::Poison(result))
}

fn default_value(ty: &ValueType) -> Result<Val, CallError> {
    Ok(match ty {
        ValueType::Bool => Val::Bool(false),
        ValueType::S8 => Val::S8(0),
        ValueType::U8 => Val::U8(0),
        ValueType::S16 => Val::S16(0),
        ValueType::U16 => Val::U16(0),
        ValueType::S32 => Val::S32(0),
        ValueType::U32 => Val::U32(0),
        ValueType::S64 => Val::S64(0),
        ValueType::U64 => Val::U64(0),
        ValueType::F32 => Val::F32(0.0),
        ValueType::F64 => Val::F64(0.0),
        ValueType::Char => Val::Char('\0'),
        ValueType::String => Val::String(String::new()),
        ValueType::List(element) if **element == ValueType::U8 => Val::Bytes(Vec::new()),
        ValueType::List(_) => Val::List(Vec::new()),
        ValueType::Tuple(types) => {
            Val::Tuple(types.iter().map(default_value).collect::<Result<_, _>>()?)
        }
        ValueType::Record(fields) => Val::Record(
            fields
                .iter()
                .map(|field| Ok((field.name.clone(), default_value(&field.ty)?)))
                .collect::<Result<_, CallError>>()?,
        ),
        ValueType::Variant(cases) => {
            let case = cases
                .first()
                .ok_or_else(|| CallError::trap("variant has no cases"))?;
            Val::Variant {
                case: case.name.clone(),
                value: case
                    .ty
                    .as_ref()
                    .map(default_value)
                    .transpose()?
                    .map(Box::new),
            }
        }
        ValueType::Enum(cases) => Val::Enum(
            cases
                .first()
                .ok_or_else(|| CallError::trap("enum has no cases"))?
                .clone(),
        ),
        ValueType::Flags(_) => Val::Flags(Vec::new()),
        ValueType::Option(_) => Val::Option(None),
        ValueType::Result { ok, .. } => Val::Result(Ok(ok
            .as_deref()
            .map(default_value)
            .transpose()?
            .map(Box::new))),
        ValueType::Stream(_) => return Err(unsupported(ty.name())),
        ValueType::Future => return Err(unsupported(ty.name())),
        ValueType::Resource(resource) => Val::Resource(match resource.ownership {
            ResourceOwnership::Own => {
                Resource::owned(resource.interface.clone(), resource.name.clone(), u32::MAX)
            }
            ResourceOwnership::Borrow => {
                Resource::borrowed(resource.interface.clone(), resource.name.clone(), u32::MAX)
            }
        }),
        ValueType::Unsupported(_) => return Err(unsupported(ty.name())),
    })
}

pub(crate) fn lower_result_tracked(
    values: &Vals,
    signature: &FunctionType,
    resources: &ResourceTracker,
) -> Result<JsResult, CallError> {
    match (values.as_slice(), &signature.result) {
        ([], None) => Ok(JsResult::Return(JsValue::UNDEFINED)),
        ([Val::Result(value)], Some(ValueType::Result { ok, err })) => {
            let (result, ty, throws) = match value {
                Ok(value) => (value, ok.as_deref(), false),
                Err(value) => (value, err.as_deref(), true),
            };
            let payload = lower_optional_payload(result.as_deref(), ty, resources)?;
            Ok(if throws {
                JsResult::Throw(payload)
            } else {
                JsResult::Return(payload)
            })
        }
        ([value], Some(ty)) => {
            lower(value.clone(), ty, ResourceRetention::Track(resources)).map(JsResult::Return)
        }
        _ => Err(CallError::trap("imported result count mismatch")),
    }
}

#[cfg(test)]
pub(crate) fn lift_result(value: JsValue, signature: &FunctionType) -> Result<Vals, CallError> {
    lift_result_tracked(value, signature, &ResourceTracker::default())
}

pub(crate) fn lift_result_tracked(
    value: JsValue,
    signature: &FunctionType,
    resources: &ResourceTracker,
) -> Result<Vals, CallError> {
    resources.refuse_guest_streams.set(true);
    let result = signature.result.as_ref().map_or_else(
        || Ok(Vec::new()),
        |ty| match ty {
            ValueType::Result { ok, .. } => lift_optional_payload(value, ok.as_deref(), resources)
                .map(|value| vec![Val::Result(Ok(value))]),
            _ => lift(value, ty, resources).map(|value| vec![value]),
        },
    );
    resources.refuse_guest_streams.set(false);
    result
}

#[cfg(test)]
pub(crate) fn lift_result_error(
    value: &JsValue,
    signature: &FunctionType,
) -> Result<Option<Vals>, CallError> {
    lift_result_error_tracked(value, signature, &ResourceTracker::default())
}

pub(crate) fn lift_result_error_tracked(
    value: &JsValue,
    signature: &FunctionType,
    resources: &ResourceTracker,
) -> Result<Option<Vals>, CallError> {
    let Some(ValueType::Result { err, .. }) = &signature.result else {
        return Ok(None);
    };
    // jco generates `ComponentError` per component, so Rust cannot name its class.
    if !Reflect::has(value, &"payload".into()).map_err(|error| {
        CallError::trap(format!("could not inspect jco result error: {error:?}"))
    })? {
        return Ok(None);
    }
    let payload = Reflect::get(value, &"payload".into())
        .map_err(|error| CallError::trap(format!("could not read jco result error: {error:?}")))?;
    lift_optional_payload(payload, err.as_deref(), resources)
        .map(|value| Some(vec![Val::Result(Err(value))]))
}

fn lower_optional_payload(
    value: Option<&Val>,
    ty: Option<&ValueType>,
    resources: &ResourceTracker,
) -> Result<JsValue, CallError> {
    match (value, ty) {
        (Some(value), Some(ty)) => lower(value.clone(), ty, ResourceRetention::Track(resources)),
        (None, None) => Ok(JsValue::UNDEFINED),
        _ => Err(CallError::trap("wrong payload for top-level WIT result")),
    }
}

fn lift_optional_payload(
    value: JsValue,
    ty: Option<&ValueType>,
    resources: &ResourceTracker,
) -> Result<Option<Box<Val>>, CallError> {
    ty.map(|ty| lift(value, ty, resources).map(Box::new))
        .transpose()
}

fn lower(
    value: Val,
    expected: &ValueType,
    resources: ResourceRetention<'_>,
) -> Result<JsValue, CallError> {
    let value = match (value, expected) {
        (Val::Bool(value), ValueType::Bool) => JsValue::from_bool(value),
        (Val::S8(value), ValueType::S8) => JsValue::from_f64(f64::from(value)),
        (Val::U8(value), ValueType::U8) => JsValue::from_f64(f64::from(value)),
        (Val::S16(value), ValueType::S16) => JsValue::from_f64(f64::from(value)),
        (Val::U16(value), ValueType::U16) => JsValue::from_f64(f64::from(value)),
        (Val::S32(value), ValueType::S32) => JsValue::from_f64(f64::from(value)),
        (Val::U32(value), ValueType::U32) => JsValue::from_f64(f64::from(value)),
        (Val::S64(value), ValueType::S64) => BigInt::from(value).into(),
        (Val::U64(value), ValueType::U64) => BigInt::from(value).into(),
        (Val::F32(value), ValueType::F32) => JsValue::from_f64(f64::from(value)),
        (Val::F64(value), ValueType::F64) => JsValue::from_f64(value),
        (Val::Char(value), ValueType::Char) => JsValue::from_str(&value.to_string()),
        (Val::String(value), ValueType::String) => JsValue::from_str(&value),
        (Val::Bytes(values), ValueType::List(element)) if **element == ValueType::U8 => {
            Uint8Array::from(values.as_slice()).into()
        }
        (value, ValueType::List(element)) if **element == ValueType::U8 => {
            return Err(wrong_val_type(expected, &value));
        }
        (Val::List(values), ValueType::List(element)) => {
            lower_sequence(values, element, resources)?
        }
        (Val::Tuple(values), ValueType::Tuple(elements)) => {
            if values.len() != elements.len() {
                return Err(mismatch(
                    expected,
                    &Val::Tuple(values),
                    "wrong element count",
                ));
            }
            values
                .into_iter()
                .zip(elements)
                .map(|(value, element)| lower(value, element, resources))
                .collect::<Result<Array, _>>()?
                .into()
        }
        (Val::Record(values), ValueType::Record(fields)) => {
            lower_record(values, fields, expected, resources)?
        }
        (Val::Variant { case, value }, ValueType::Variant(cases)) => {
            lower_variant(case, value, cases, expected, resources)?
        }
        (Val::Enum(case), ValueType::Enum(cases)) if cases.contains(&case) => {
            JsValue::from_str(&case)
        }
        (Val::Flags(names), ValueType::Flags(flags)) => lower_flags(names, flags, expected)?,
        (Val::Option(value), ValueType::Option(payload)) => {
            lower_option(value, payload, expected, resources)?
        }
        (Val::Result(value), ValueType::Result { ok, err }) => {
            lower_nested_result(value, ok.as_deref(), err.as_deref(), expected, resources)?
        }
        (Val::Resource(resource), ValueType::Resource(expected)) => {
            lower_resource(resource, expected, resources)?
        }
        (Val::Stream(handle), ValueType::Stream(item)) => {
            if **item != ValueType::U8 || !handle.is_byte_stream() {
                return Err(CallError::refused(
                    "jco does not yet support WIT value streams",
                ));
            }
            let ResourceRetention::Track(resources) = resources else {
                return Err(CallError::trap("cannot synthesize a byte stream"));
            };
            let id = handle.id();
            let input = InputStream::try_from(handle)
                .map_err(|error| CallError::trap(error.to_string()))?;
            resources.host_streams.borrow_mut().insert(id, input);
            resources.channel_open(id, ChannelDirection::HostToGuest);
            stream_marker("host", id)
        }
        (_, ValueType::Stream(_)) => return Err(unsupported(expected.name())),
        (_, ValueType::Future) => return Err(unsupported(expected.name())),
        (_, ValueType::Unsupported(name)) => return Err(unsupported(name)),
        (value, expected) => {
            return Err(wrong_val_type(expected, &value));
        }
    };
    Ok(value)
}

fn lift(
    value: JsValue,
    expected: &ValueType,
    resources: &ResourceTracker,
) -> Result<Val, CallError> {
    macro_rules! number {
        ($ty:ty, $variant:ident) => {
            number(&value, expected)
                .and_then(|value| integer::<$ty>(value, expected))
                .map(Val::$variant)
        };
    }
    match expected {
        ValueType::Bool => value
            .as_bool()
            .map(Val::Bool)
            .ok_or_else(|| wrong_js_type(expected, &value)),
        ValueType::S8 => number!(i8, S8),
        ValueType::U8 => number!(u8, U8),
        ValueType::S16 => number!(i16, S16),
        ValueType::U16 => number!(u16, U16),
        ValueType::S32 => number!(i32, S32),
        ValueType::U32 => number!(u32, U32),
        ValueType::S64 => bigint::<i64>(value, expected).map(Val::S64),
        ValueType::U64 => bigint::<u64>(value, expected).map(Val::U64),
        ValueType::F32 => number(&value, expected).map(|value| Val::F32(value as f32)),
        ValueType::F64 => number(&value, expected).map(Val::F64),
        ValueType::Char => value
            .as_string()
            .and_then(one_char)
            .map(Val::Char)
            .ok_or_else(|| mismatch(expected, &value, "expected one Unicode scalar value")),
        ValueType::String => value
            .as_string()
            .map(Val::String)
            .ok_or_else(|| wrong_js_type(expected, &value)),
        ValueType::List(element) if **element == ValueType::U8 => value
            .dyn_into::<Uint8Array>()
            .map(|values| Val::Bytes(values.to_vec()))
            .map_err(|value| wrong_js_type(expected, &value)),
        ValueType::List(element) => lift_sequence(value, element, resources).map(Val::List),
        ValueType::Tuple(elements) => {
            let values = js_array(value, expected)?;
            if values.length() as usize != elements.len() {
                return Err(mismatch(expected, &values, "wrong element count"));
            }
            elements
                .iter()
                .enumerate()
                .map(|(index, element)| lift(values.get(index as u32), element, resources))
                .collect::<Result<Vec<_>, _>>()
                .map(Val::Tuple)
        }
        ValueType::Record(fields) => lift_record(value, fields, expected, resources),
        ValueType::Variant(cases) => lift_variant(value, cases, expected, resources),
        ValueType::Enum(cases) => value
            .as_string()
            .filter(|case| cases.contains(case))
            .map(Val::Enum)
            .ok_or_else(|| mismatch(expected, &value, "unknown enum case")),
        ValueType::Flags(flags) => lift_flags(value, flags, expected),
        ValueType::Option(payload) => lift_option(value, payload, expected, resources),
        ValueType::Result { ok, err } => {
            lift_nested_result(value, ok.as_deref(), err.as_deref(), expected, resources)
        }
        ValueType::Resource(expected) => lift_resource(value, expected, resources),
        ValueType::Stream(item) if **item == ValueType::U8 => {
            lift_stream(value, expected, resources)
        }
        ValueType::Stream(_) => refuse_value_stream(value, expected, resources),
        ValueType::Future => Err(unsupported(expected.name())),
        ValueType::Unsupported(name) => Err(unsupported(name)),
    }
}

fn stream_marker(kind: &str, id: u64) -> JsValue {
    let marker = Object::new();
    let description = Array::of2(&kind.into(), &BigInt::from(id).into());
    let _ = Reflect::set(&marker, &STREAM_MARKER.into(), &description);
    marker.into()
}

fn lift_stream(
    value: JsValue,
    expected: &ValueType,
    resources: &ResourceTracker,
) -> Result<Val, CallError> {
    let (kind, id) = stream_identity(&value, expected)?;
    let handle = match kind.as_str() {
        "host" => resources.take_host(id).map(InputStream::into_handle),
        "guest" if resources.refuse_guest_streams.get() => {
            let handle = resources.take_guest(id).ok_or_else(|| {
                CallError::trap(format!("stream `guest#{id}` is no longer available"))
            })?;
            handle.__close_reader();
            resources.close_guest(id);
            return Err(CallError::refused(
                "guest-created streams cannot be returned because the component store ends with each call",
            ));
        }
        "guest" => resources.take_guest(id),
        _ => None,
    };
    handle
        .map(Val::Stream)
        .ok_or_else(|| CallError::trap(format!("stream `{kind}#{id}` is no longer available")))
}

fn refuse_value_stream(
    value: JsValue,
    expected: &ValueType,
    resources: &ResourceTracker,
) -> Result<Val, CallError> {
    let (kind, id) = stream_identity(&value, expected)?;
    match kind.as_str() {
        "host" => resources
            .take_host(id)
            .ok_or_else(|| CallError::trap(format!("stream `host#{id}` is no longer available")))?
            .close_reader(),
        "guest" => {
            let handle = resources.take_guest(id).ok_or_else(|| {
                CallError::trap(format!("stream `guest#{id}` is no longer available"))
            })?;
            handle.__close_reader();
            resources.close_guest(id);
        }
        _ => {
            return Err(CallError::trap(format!(
                "stream `{kind}#{id}` is no longer available"
            )));
        }
    }
    Err(CallError::refused(
        "jco does not yet support WIT value streams",
    ))
}

fn stream_identity(value: &JsValue, expected: &ValueType) -> Result<(String, u64), CallError> {
    let marker = Reflect::get(&value, &STREAM_MARKER.into())
        .map_err(|error| mismatch(expected, &error, "could not read stream marker"))?;
    let marker = js_array(marker, expected)?;
    let kind = marker
        .get(0)
        .as_string()
        .ok_or_else(|| mismatch(expected, &value, "missing stream direction"))?;
    let id = bigint::<u64>(marker.get(1), &ValueType::U64)?;
    Ok((kind, id))
}

fn lower_sequence(
    values: Vec<Val>,
    element: &ValueType,
    resources: ResourceRetention<'_>,
) -> Result<JsValue, CallError> {
    values
        .into_iter()
        .map(|value| lower(value, element, resources))
        .collect::<Result<Array, _>>()
        .map(Into::into)
}

fn lift_sequence(
    value: JsValue,
    element: &ValueType,
    resources: &ResourceTracker,
) -> Result<Vec<Val>, CallError> {
    js_array(value, &ValueType::List(Box::new(element.clone())))?
        .iter()
        .map(|value| lift(value, element, resources))
        .collect()
}

fn js_array(value: JsValue, expected: &ValueType) -> Result<Array, CallError> {
    if Array::is_array(&value) {
        Ok(Array::from(&value))
    } else {
        Err(wrong_js_type(expected, &value))
    }
}

fn lower_record(
    mut values: Vec<(String, Val)>,
    fields: &[crate::types::FieldType],
    expected: &ValueType,
    resources: ResourceRetention<'_>,
) -> Result<JsValue, CallError> {
    for field in fields {
        if !values.iter().any(|(name, _)| name == &field.name) {
            return Err(mismatch(
                expected,
                &Val::Record(values),
                &format!("missing field `{}`", field.name),
            ));
        }
    }
    if values.len() != fields.len()
        || values
            .iter()
            .any(|(name, _)| !fields.iter().any(|field| &field.name == name))
    {
        return Err(mismatch(
            expected,
            &Val::Record(values),
            "unknown or duplicate field",
        ));
    }
    let object = Object::new();
    for field in fields {
        let index = values
            .iter()
            .position(|(name, _)| name == &field.name)
            .ok_or_else(|| mismatch(expected, &values, "record changed during conversion"))?;
        let (_, value) = values.remove(index);
        Reflect::set(
            &object,
            &JsValue::from_str(&field.js_name),
            &lower(value, &field.ty, resources)?,
        )
        .map_err(|error| mismatch(expected, &error, "could not set record field"))?;
    }
    Ok(object.into())
}

fn lift_record(
    value: JsValue,
    fields: &[crate::types::FieldType],
    expected: &ValueType,
    resources: &ResourceTracker,
) -> Result<Val, CallError> {
    if !value.is_object() || Array::is_array(&value) {
        return Err(wrong_js_type(expected, &value));
    }
    let object = Object::from(value.clone());
    for key in Object::keys(&object)
        .iter()
        .filter_map(|key| key.as_string())
    {
        if !fields.iter().any(|field| field.js_name == key) {
            return Err(mismatch(
                expected,
                &value,
                &format!("unknown field `{key}`"),
            ));
        }
    }
    let values = fields
        .iter()
        .map(|field| {
            let key = JsValue::from_str(&field.js_name);
            let present = Reflect::has(&object, &key)
                .map_err(|error| mismatch(expected, &error, "could not inspect record field"))?;
            if !present && !matches!(field.ty, ValueType::Option(_)) {
                return Err(mismatch(
                    expected,
                    &value,
                    &format!("missing field `{}`", field.name),
                ));
            }
            Reflect::get(&object, &key)
                .map_err(|error| mismatch(expected, &error, "could not read record field"))
                .and_then(|value| lift(value, &field.ty, resources))
                .map(|value| (field.name.clone(), value))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Val::Record(values))
}

fn lower_variant(
    case: String,
    value: Option<Box<Val>>,
    cases: &[crate::types::CaseType],
    expected: &ValueType,
    resources: ResourceRetention<'_>,
) -> Result<JsValue, CallError> {
    let actual = Val::Variant {
        case: case.clone(),
        value: value.clone(),
    };
    let Some(case_type) = cases.iter().find(|candidate| candidate.name == case) else {
        return Err(mismatch(expected, &actual, "unknown variant case"));
    };
    let object = Object::new();
    Reflect::set(&object, &"tag".into(), &JsValue::from_str(&case))
        .map_err(|error| mismatch(expected, &error, "could not set variant tag"))?;
    match (value, &case_type.ty) {
        (None, None) => {}
        (Some(value), Some(ty)) => {
            Reflect::set(&object, &"val".into(), &lower(*value, ty, resources)?)
                .map_err(|error| mismatch(expected, &error, "could not set variant payload"))?;
        }
        _ => {
            return Err(mismatch(
                expected,
                &actual,
                "wrong payload for variant case",
            ));
        }
    }
    Ok(object.into())
}

fn lift_variant(
    value: JsValue,
    cases: &[crate::types::CaseType],
    expected: &ValueType,
    resources: &ResourceTracker,
) -> Result<Val, CallError> {
    if !value.is_object() || Array::is_array(&value) {
        return Err(wrong_js_type(expected, &value));
    }
    let tag = Reflect::get(&value, &"tag".into())
        .ok()
        .and_then(|tag| tag.as_string())
        .ok_or_else(|| mismatch(expected, &value, "missing string tag"))?;
    let case = cases
        .iter()
        .find(|candidate| candidate.name == tag)
        .ok_or_else(|| mismatch(expected, &value, "unknown variant case"))?;
    let payload = case
        .ty
        .as_ref()
        .map(|ty| {
            Reflect::get(&value, &"val".into())
                .map_err(|error| mismatch(expected, &error, "could not read variant payload"))
                .and_then(|value| lift(value, ty, resources))
                .map(Box::new)
        })
        .transpose()?;
    Ok(Val::Variant {
        case: tag,
        value: payload,
    })
}

fn lower_flags(
    names: Vec<String>,
    flags: &[String],
    expected: &ValueType,
) -> Result<JsValue, CallError> {
    if names.iter().any(|name| !flags.contains(name)) {
        return Err(mismatch(expected, &Val::Flags(names), "unknown flag"));
    }
    let object = Object::new();
    for flag in flags {
        Reflect::set(
            &object,
            &JsValue::from_str(&crate::types::js_name(flag)),
            &JsValue::from_bool(names.contains(flag)),
        )
        .map_err(|error| mismatch(expected, &error, "could not set flag"))?;
    }
    Ok(object.into())
}

fn lift_flags(value: JsValue, flags: &[String], expected: &ValueType) -> Result<Val, CallError> {
    if !value.is_object() || Array::is_array(&value) {
        return Err(wrong_js_type(expected, &value));
    }
    let object = Object::from(value.clone());
    for key in Object::keys(&object)
        .iter()
        .filter_map(|key| key.as_string())
    {
        if !flags.iter().any(|flag| crate::types::js_name(flag) == key) {
            return Err(mismatch(expected, &value, &format!("unknown flag `{key}`")));
        }
    }
    let mut names = Vec::new();
    for flag in flags {
        let key = JsValue::from_str(&crate::types::js_name(flag));
        if Reflect::has(&object, &key)
            .map_err(|error| mismatch(expected, &error, "could not inspect flag"))?
        {
            let active = Reflect::get(&object, &key)
                .map_err(|error| mismatch(expected, &error, "could not read flag"))?
                .as_bool()
                .ok_or_else(|| mismatch(expected, &value, "flag was not a boolean"))?;
            if active {
                names.push(flag.clone());
            }
        }
    }
    Ok(Val::Flags(names))
}

fn lower_option(
    value: Option<Box<Val>>,
    payload: &ValueType,
    expected: &ValueType,
    resources: ResourceRetention<'_>,
) -> Result<JsValue, CallError> {
    match (value, maybe_null(payload)) {
        (None, false) => Ok(JsValue::UNDEFINED),
        (Some(value), false) => lower(*value, payload, resources),
        (None, true) => tagged("none", None, expected),
        (Some(value), true) => tagged("some", Some(lower(*value, payload, resources)?), expected),
    }
}

fn lift_option(
    value: JsValue,
    payload: &ValueType,
    expected: &ValueType,
    resources: &ResourceTracker,
) -> Result<Val, CallError> {
    if !maybe_null(payload) {
        return if value.is_null() || value.is_undefined() {
            Ok(Val::Option(None))
        } else {
            lift(value, payload, resources).map(|value| Val::Option(Some(Box::new(value))))
        };
    }
    let (tag, value) = tagged_parts(&value, expected)?;
    match tag.as_str() {
        "none" => Ok(Val::Option(None)),
        "some" => lift(value, payload, resources).map(|value| Val::Option(Some(Box::new(value)))),
        _ => Err(mismatch(expected, &value, "unknown option case")),
    }
}

fn maybe_null(ty: &ValueType) -> bool {
    matches!(ty, ValueType::Option(payload) if !maybe_null(payload))
}

fn tagged(tag: &str, value: Option<JsValue>, expected: &ValueType) -> Result<JsValue, CallError> {
    let object = Object::new();
    Reflect::set(&object, &"tag".into(), &JsValue::from_str(tag))
        .map_err(|error| mismatch(expected, &error, "could not set tag"))?;
    if let Some(value) = value {
        Reflect::set(&object, &"val".into(), &value)
            .map_err(|error| mismatch(expected, &error, "could not set payload"))?;
    }
    Ok(object.into())
}

fn tagged_parts(value: &JsValue, expected: &ValueType) -> Result<(String, JsValue), CallError> {
    if !value.is_object() || Array::is_array(value) {
        return Err(wrong_js_type(expected, value));
    }
    let tag = Reflect::get(value, &"tag".into())
        .ok()
        .and_then(|tag| tag.as_string())
        .ok_or_else(|| mismatch(expected, value, "missing string tag"))?;
    let payload = Reflect::get(value, &"val".into())
        .map_err(|error| mismatch(expected, &error, "could not read payload"))?;
    Ok((tag, payload))
}

fn lower_nested_result(
    value: Result<Option<Box<Val>>, Option<Box<Val>>>,
    ok: Option<&ValueType>,
    err: Option<&ValueType>,
    expected: &ValueType,
    resources: ResourceRetention<'_>,
) -> Result<JsValue, CallError> {
    let (tag, value, ty) = match value {
        Ok(value) => ("ok", value, ok),
        Err(value) => ("err", value, err),
    };
    let payload = match (value, ty) {
        (Some(value), Some(ty)) => lower(*value, ty, resources)?,
        (None, None) => JsValue::UNDEFINED,
        (value, _) => {
            return Err(mismatch(expected, &value, "wrong payload for result case"));
        }
    };
    tagged(tag, Some(payload), expected)
}

fn lift_nested_result(
    value: JsValue,
    ok: Option<&ValueType>,
    err: Option<&ValueType>,
    expected: &ValueType,
    resources: &ResourceTracker,
) -> Result<Val, CallError> {
    let (tag, payload) = tagged_parts(&value, expected)?;
    let lift_payload = |ty: Option<&ValueType>| {
        ty.map(|ty| lift(payload.clone(), ty, resources).map(Box::new))
            .transpose()
    };
    match tag.as_str() {
        "ok" => lift_payload(ok).map(|value| Val::Result(Ok(value))),
        "err" => lift_payload(err).map(|value| Val::Result(Err(value))),
        _ => Err(mismatch(expected, &value, "unknown result case")),
    }
}

fn lower_resource(
    resource: Resource,
    expected: &ResourceType,
    resources: ResourceRetention<'_>,
) -> Result<JsValue, CallError> {
    let retain = validate_resource_lowering(
        &resource,
        &expected.interface,
        &expected.name,
        expected.ownership,
    )?;
    if retain && let ResourceRetention::Track(resources) = resources {
        resources.retain(resource.clone());
    }
    let descriptor = Array::of3(
        &JsValue::from_str(resource.interface()),
        &JsValue::from_str(resource.name()),
        &JsValue::from_f64(f64::from(resource.id())),
    );
    let value = Object::new();
    Reflect::set(&value, &RESOURCE_MARKER.into(), &descriptor)
        .map_err(|error| resource_mismatch(expected, &error, "could not create resource"))?;
    Ok(value.into())
}

fn lift_resource(
    value: JsValue,
    expected: &ResourceType,
    resources: &ResourceTracker,
) -> Result<Val, CallError> {
    let id = Reflect::get(&value, &"id".into())
        .map_err(|error| resource_mismatch(expected, &error, "could not read resource id"))?;
    let id = integer::<u32>(number(&id, &ValueType::U32)?, &ValueType::U32)?;
    let resource = if expected.ownership == ResourceOwnership::Own {
        resources.take(&expected.interface, &expected.name, id)?
    } else {
        Resource::borrowed(expected.interface.clone(), expected.name.clone(), id)
    };
    Ok(Val::Resource(resource))
}

fn resource_mismatch(
    expected: &ResourceType,
    value: &impl std::fmt::Debug,
    reason: &str,
) -> CallError {
    mismatch(&ValueType::Resource(expected.clone()), value, reason)
}

fn one_char(value: String) -> Option<char> {
    let mut characters = value.chars();
    let character = characters.next()?;
    characters.next().is_none().then_some(character)
}

fn number(value: &JsValue, expected: &ValueType) -> Result<f64, CallError> {
    value.as_f64().ok_or_else(|| wrong_js_type(expected, value))
}

#[allow(clippy::cast_possible_truncation)]
fn integer<T>(value: f64, expected: &ValueType) -> Result<T, CallError>
where
    T: TryFrom<i64>,
{
    if !value.is_finite() || value.fract() != 0.0 {
        return Err(mismatch(expected, &value, "expected an integer"));
    }
    T::try_from(value as i64).map_err(|_| mismatch(expected, &value, "integer is out of range"))
}

fn bigint<T>(value: JsValue, expected: &ValueType) -> Result<T, CallError>
where
    T: TryFrom<BigInt>,
{
    value
        .dyn_into::<BigInt>()
        .map_err(|value| wrong_js_type(expected, &value))
        .and_then(|value| {
            T::try_from(value.clone())
                .map_err(|_| mismatch(expected, &value, "integer is out of range"))
        })
}

fn wrong_val_type(expected: &ValueType, value: &Val) -> CallError {
    mismatch(expected, value, "wrong framework value kind")
}

fn wrong_js_type(expected: &ValueType, value: &JsValue) -> CallError {
    mismatch(expected, value, "wrong JavaScript value kind")
}

fn mismatch(expected: &ValueType, value: &impl std::fmt::Debug, reason: &str) -> CallError {
    CallError::trap(format!(
        "invalid WIT `{}` value {value:?}: {reason}",
        expected.name()
    ))
}

fn unsupported(name: &str) -> CallError {
    CallError::trap(format!("jco does not yet support WIT `{name}` values"))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};
    use wasm_junction_core::{BoxFuture, CallErrorKind, ImportTarget, InvocationContext, Resource};

    use super::*;

    wasm_bindgen_test_configure!(run_in_dedicated_worker);

    #[derive(Default)]
    struct EventDispatcher(RefCell<Vec<EngineEvent>>);

    impl ImportDispatcher for EventDispatcher {
        fn call(
            &self,
            _context: InvocationContext,
            _caller: Arc<str>,
            _interface: Arc<str>,
            _function: Arc<str>,
            _args: Vals,
        ) -> BoxFuture<'_, Result<Vals, CallError>> {
            Box::pin(std::future::ready(Err(CallError::trap("unused call"))))
        }

        fn call_engine(
            &self,
            _context: InvocationContext,
            _caller: Arc<str>,
            _interface: Arc<str>,
            _function: Arc<str>,
            _args: Vals,
            _target: Arc<dyn ImportTarget>,
        ) -> BoxFuture<'_, Result<Vals, CallError>> {
            Box::pin(std::future::ready(Err(CallError::trap(
                "unused engine call",
            ))))
        }

        fn drop_resource(
            &self,
            _context: InvocationContext,
            _caller: Arc<str>,
            _resource: Resource,
        ) -> BoxFuture<'_, Result<(), CallError>> {
            Box::pin(std::future::ready(Err(CallError::trap("unused drop"))))
        }

        fn emit(&self, event: EngineEvent) {
            self.0.borrow_mut().push(event);
        }
    }

    #[wasm_bindgen_test]
    fn round_trips_primitive_values() {
        let (values, params): (Vals, Vec<ValueType>) = [
            (Val::Bool(true), ValueType::Bool),
            (Val::S8(-8), ValueType::S8),
            (Val::U8(8), ValueType::U8),
            (Val::S16(-16), ValueType::S16),
            (Val::U16(16), ValueType::U16),
            (Val::S32(-32), ValueType::S32),
            (Val::U32(32), ValueType::U32),
            (Val::S64(-64), ValueType::S64),
            (Val::U64(64), ValueType::U64),
            (Val::F32(3.5), ValueType::F32),
            (Val::F64(7.25), ValueType::F64),
            (Val::Char('§'), ValueType::Char),
            (Val::String("value".to_owned()), ValueType::String),
        ]
        .into_iter()
        .unzip();
        let signature = FunctionType {
            params,
            result: None,
        };
        let lowered = lower_args(values.clone(), &signature).unwrap();
        assert_eq!(lift_args(&lowered, &signature).unwrap(), values);
        let result = vec![Val::String("result".to_owned())];
        let signature = FunctionType {
            params: Vec::new(),
            result: Some(ValueType::String),
        };
        let JsResult::Return(lowered) = lower_result(&result, &signature).unwrap() else {
            panic!("plain result unexpectedly threw")
        };
        assert_eq!(lift_result(lowered, &signature).unwrap(), result);
        let signature = FunctionType {
            params: vec![ValueType::Unsupported("record")],
            result: None,
        };
        let error = lower_args(vec![Val::Record(Vec::new())], &signature).unwrap_err();
        assert_eq!(
            error.to_string(),
            "jco does not yet support WIT `record` values"
        );
    }

    #[wasm_bindgen_test]
    fn round_trips_lists_byte_lists_and_tuples() {
        let values = vec![
            Val::List(vec![Val::from("rust"), Val::from("wasm")]),
            Val::Bytes(vec![1, 2]),
            Val::Tuple(vec![Val::S32(-71), Val::S32(42)]),
        ];
        let signature = FunctionType {
            params: vec![
                ValueType::List(Box::new(ValueType::String)),
                ValueType::List(Box::new(ValueType::U8)),
                ValueType::Tuple(vec![ValueType::S32, ValueType::S32]),
            ],
            result: None,
        };
        let lowered = lower_args(values.clone(), &signature).unwrap();
        assert!(lowered.get(1).is_instance_of::<Uint8Array>());
        assert_eq!(lift_args(&lowered, &signature).unwrap(), values);

        let error = lower_args(
            vec![Val::List(vec![Val::U8(1)])],
            &FunctionType {
                params: vec![ValueType::List(Box::new(ValueType::U8))],
                result: None,
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("WIT `list`"), "{error}");
    }

    #[wasm_bindgen_test]
    async fn recovers_host_streams_and_refuses_guest_stream_results() {
        let tracker = ResourceTracker::default();
        let signature = FunctionType {
            params: vec![ValueType::Stream(Box::new(ValueType::U8))],
            result: None,
        };
        let lowered = lower_args_tracked(
            vec![wasm_junction_core::OutputStream::from_bytes(b"host").into()],
            &signature,
            &tracker,
        )
        .unwrap();
        let mut values = lift_args_tracked(&lowered, &signature, &tracker).unwrap();
        let input = InputStream::try_from(values.remove(0)).unwrap();
        assert_eq!(input.read_all().await.unwrap(), b"host");

        let (_, output) = wasm_junction_core::OutputStream::<u8>::channel();
        let marker = tracker.register_guest(StreamHandle::from(output));
        let error = lift_result_tracked(
            marker,
            &FunctionType {
                params: Vec::new(),
                result: Some(ValueType::Stream(Box::new(ValueType::U8))),
            },
            &tracker,
        )
        .unwrap_err();
        assert!(error.to_string().contains("store ends with each call"));
    }

    #[wasm_bindgen_test]
    async fn refuses_value_streams_without_panicking() {
        let error = lower_args_tracked(
            vec![wasm_junction_core::OutputStream::from_items([7_u32]).into()],
            &FunctionType {
                params: vec![ValueType::Stream(Box::new(ValueType::U32))],
                result: None,
            },
            &ResourceTracker::default(),
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "jco does not yet support WIT value streams"
        );

        let dispatcher = Arc::new(EventDispatcher::default());
        let tracker = ResourceTracker::with_imports(
            dispatcher.clone(),
            Some(InvocationId::__from_counter(7)),
        );
        let (writer, output) = wasm_junction_core::OutputStream::<u32>::channel();
        let marker = tracker.register_guest(StreamHandle::from(output));
        let error = lift_result_tracked(
            marker,
            &FunctionType {
                params: Vec::new(),
                result: Some(ValueType::Stream(Box::new(ValueType::U32))),
            },
            &tracker,
        )
        .unwrap_err();
        assert_eq!(error.kind(), CallErrorKind::Refused);
        assert_eq!(
            error.to_string(),
            "jco does not yet support WIT value streams"
        );
        assert_eq!(
            writer.write([8]).await.unwrap_err().to_string(),
            "stream reader is closed"
        );
        let events = dispatcher.0.borrow();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, EngineEvent::ChannelOpen { .. }))
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, EngineEvent::ChannelClose { .. }))
                .count(),
            1
        );
        drop(events);

        assert_eq!(
            lift_result_tracked(
                JsValue::from_f64(9.0),
                &FunctionType {
                    params: Vec::new(),
                    result: Some(ValueType::U32),
                },
                &tracker,
            )
            .unwrap(),
            [Val::U32(9)]
        );
    }

    #[wasm_bindgen_test]
    fn refuses_component_future_values() {
        let future =
            wasm_junction_core::FutureHandle::__for_invocation(7, InvocationId::__from_counter(3));
        let error = lower_args(
            vec![Val::Future(future)],
            &FunctionType {
                params: vec![ValueType::Future],
                result: None,
            },
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "jco does not yet support WIT `future` values"
        );
    }

    #[wasm_bindgen_test]
    fn round_trips_records_with_jco_field_names() {
        let ty = ValueType::Record(vec![
            crate::types::FieldType {
                name: "signed-8".to_owned(),
                js_name: "signed8".to_owned(),
                ty: ValueType::S8,
            },
            crate::types::FieldType {
                name: "tags".to_owned(),
                js_name: "tags".to_owned(),
                ty: ValueType::List(Box::new(ValueType::String)),
            },
        ]);
        let values = vec![Val::Record(vec![
            ("signed-8".to_owned(), Val::S8(-8)),
            ("tags".to_owned(), Val::List(vec![Val::from("wasm")])),
        ])];
        let signature = FunctionType {
            params: vec![ty],
            result: None,
        };
        let lowered = lower_args(values.clone(), &signature).unwrap();
        assert_eq!(
            Reflect::get(&lowered.get(0), &"signed8".into())
                .unwrap()
                .as_f64(),
            Some(-8.0)
        );
        assert_eq!(lift_args(&lowered, &signature).unwrap(), values);
    }

    #[wasm_bindgen_test]
    fn refuses_a_missing_record_field() {
        let error = lift_error(
            Object::new().into(),
            ValueType::Record(vec![crate::types::FieldType {
                name: "title".to_owned(),
                js_name: "title".to_owned(),
                ty: ValueType::String,
            }]),
        );
        assert!(error.contains("WIT `record`"), "{error}");
        assert!(error.contains("missing field `title`"), "{error}");
    }

    #[wasm_bindgen_test]
    fn round_trips_variants_and_enums() {
        let cases = vec![
            crate::types::CaseType {
                name: "none".to_owned(),
                ty: None,
            },
            crate::types::CaseType {
                name: "text".to_owned(),
                ty: Some(ValueType::String),
            },
        ];
        let values = vec![
            Val::Variant {
                case: "text".to_owned(),
                value: Some(Box::new(Val::from("diagram"))),
            },
            Val::Enum("upbeat".to_owned()),
        ];
        let signature = FunctionType {
            params: vec![
                ValueType::Variant(cases),
                ValueType::Enum(vec!["neutral".to_owned(), "upbeat".to_owned()]),
            ],
            result: None,
        };
        let lowered = lower_args(values.clone(), &signature).unwrap();
        assert_eq!(lift_args(&lowered, &signature).unwrap(), values);
    }

    #[wasm_bindgen_test]
    fn refuses_an_unknown_variant_case() {
        let error = lower_args(
            vec![Val::Variant {
                case: "missing".to_owned(),
                value: None,
            }],
            &FunctionType {
                params: vec![ValueType::Variant(Vec::new())],
                result: None,
            },
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("WIT `variant`"), "{error}");
        assert!(error.contains("missing"), "{error}");
        assert!(error.contains("unknown variant case"), "{error}");
    }

    #[wasm_bindgen_test]
    fn round_trips_flags() {
        let values = vec![Val::Flags(vec![
            "short-form".to_owned(),
            "detailed".to_owned(),
        ])];
        let signature = FunctionType {
            params: vec![ValueType::Flags(vec![
                "short-form".to_owned(),
                "detailed".to_owned(),
            ])],
            result: None,
        };
        let lowered = lower_args(values.clone(), &signature).unwrap();
        assert_eq!(
            Reflect::get(&lowered.get(0), &"shortForm".into())
                .unwrap()
                .as_bool(),
            Some(true)
        );
        assert_eq!(lift_args(&lowered, &signature).unwrap(), values);
    }

    #[wasm_bindgen_test]
    fn refuses_an_unknown_flag() {
        let error = lower_args(
            vec![Val::Flags(vec!["missing".to_owned()])],
            &FunctionType {
                params: vec![ValueType::Flags(vec!["known".to_owned()])],
                result: None,
            },
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("WIT `flags`"), "{error}");
        assert!(error.contains("missing"), "{error}");
        assert!(error.contains("unknown flag"), "{error}");
    }

    #[wasm_bindgen_test]
    fn round_trips_options_including_nested_options() {
        let values = vec![
            Val::Option(None),
            Val::Option(Some(Box::new(Val::from("subtitle")))),
            Val::Option(Some(Box::new(Val::Option(None)))),
            Val::Option(Some(Box::new(Val::Option(Some(Box::new(Val::U32(7))))))),
        ];
        let string = ValueType::Option(Box::new(ValueType::String));
        let nested = ValueType::Option(Box::new(ValueType::Option(Box::new(ValueType::U32))));
        let signature = FunctionType {
            params: vec![string.clone(), string, nested.clone(), nested],
            result: None,
        };
        let lowered = lower_args(values.clone(), &signature).unwrap();
        assert!(lowered.get(0).is_undefined());
        assert_eq!(
            Reflect::get(&lowered.get(2), &"tag".into())
                .unwrap()
                .as_string(),
            Some("some".to_owned())
        );
        assert_eq!(lift_args(&lowered, &signature).unwrap(), values);
    }

    #[wasm_bindgen_test]
    fn round_trips_nested_results() {
        let values = vec![
            Val::Result(Ok(Some(Box::new(Val::Option(None))))),
            Val::Result(Err(Some(Box::new(Val::from("denied"))))),
            Val::Result(Ok(None)),
        ];
        let signature = FunctionType {
            params: vec![
                ValueType::Result {
                    ok: Some(Box::new(ValueType::Option(Box::new(ValueType::U64)))),
                    err: Some(Box::new(ValueType::String)),
                },
                ValueType::Result {
                    ok: Some(Box::new(ValueType::Option(Box::new(ValueType::U64)))),
                    err: Some(Box::new(ValueType::String)),
                },
                ValueType::Result {
                    ok: None,
                    err: None,
                },
            ],
            result: None,
        };
        let lowered = lower_args(values.clone(), &signature).unwrap();
        assert_eq!(
            Reflect::get(&lowered.get(1), &"tag".into())
                .unwrap()
                .as_string(),
            Some("err".to_owned())
        );
        assert_eq!(lift_args(&lowered, &signature).unwrap(), values);
    }

    #[wasm_bindgen_test]
    fn translates_top_level_results_to_jco_exceptions() {
        let signature = FunctionType {
            params: Vec::new(),
            result: Some(ValueType::Result {
                ok: Some(Box::new(ValueType::U64)),
                err: Some(Box::new(ValueType::String)),
            }),
        };
        let JsResult::Return(ok) = lower_result(
            &vec![Val::Result(Ok(Some(Box::new(Val::U64(7)))))],
            &signature,
        )
        .unwrap() else {
            panic!("successful result threw")
        };
        assert!(ok.dyn_into::<BigInt>().is_ok());
        let JsResult::Throw(error) = lower_result(
            &vec![Val::Result(Err(Some(Box::new(Val::from("denied")))))],
            &signature,
        )
        .unwrap() else {
            panic!("error result returned")
        };
        assert_eq!(error.as_string().as_deref(), Some("denied"));
        assert_eq!(
            lift_result(BigInt::from(7_u64).into(), &signature).unwrap(),
            [Val::Result(Ok(Some(Box::new(Val::U64(7)))))]
        );
        let component_error = js_sys::Error::new("denied");
        Reflect::set(&component_error, &"payload".into(), &"denied".into()).unwrap();
        assert_eq!(
            lift_result_error(&component_error.into(), &signature).unwrap(),
            Some(vec![Val::Result(Err(Some(Box::new(Val::from("denied")))))]),
        );
    }

    #[wasm_bindgen_test]
    fn refuses_an_out_of_range_u8() {
        let error = lift_error(JsValue::from_f64(300.0), ValueType::U8);
        assert!(error.contains("WIT `u8`"), "{error}");
        assert!(error.contains("300"), "{error}");
        assert!(error.contains("out of range"), "{error}");
    }

    #[wasm_bindgen_test]
    fn refuses_a_fractional_s32() {
        let error = lift_error(JsValue::from_f64(3.5), ValueType::S32);
        assert!(error.contains("WIT `s32`"), "{error}");
        assert!(error.contains("3.5"), "{error}");
        assert!(error.contains("expected an integer"), "{error}");
    }

    #[wasm_bindgen_test]
    fn refuses_out_of_range_bigints() {
        for (value, ty) in [
            (BigInt::from(u128::MAX).into(), ValueType::U64),
            (BigInt::from(u64::MAX).into(), ValueType::S64),
        ] {
            let error = lift_error(value, ty);
            assert!(error.contains("out of range"), "{error}");
        }
    }

    #[wasm_bindgen_test]
    fn refuses_a_string_where_a_number_is_expected() {
        let error = lift_error(JsValue::from_str("three"), ValueType::U32);
        assert!(error.contains("WIT `u32`"), "{error}");
        assert!(error.contains("three"), "{error}");
    }

    #[wasm_bindgen_test]
    fn resources_validate_type_and_ownership_while_crossing_js() {
        let expected = ResourceType {
            interface: "example:resources/host@1.0.0".to_owned(),
            name: "session".to_owned(),
            ownership: ResourceOwnership::Own,
        };
        let signature = FunctionType {
            params: vec![ValueType::Resource(expected)],
            result: None,
        };
        let resources = ResourceTracker::default();
        let owned = Resource::owned("example:resources/host@1.0.0", "session", 7);
        let lowered =
            lower_args_tracked(vec![Val::Resource(owned.clone())], &signature, &resources).unwrap();
        assert!(Reflect::has(&lowered.get(0), &RESOURCE_MARKER.into()).unwrap());

        let object = Object::new();
        Reflect::set(&object, &"id".into(), &7.into()).unwrap();
        assert_eq!(
            lift_args_tracked(&Array::of1(&object), &signature, &resources).unwrap(),
            [Val::Resource(owned)]
        );
        assert!(
            resources
                .take("example:resources/host@1.0.0", "session", 7)
                .is_err()
        );

        let borrowed = Resource::borrowed("example:resources/host@1.0.0", "session", 8);
        let error =
            lower_args_tracked(vec![Val::Resource(borrowed)], &signature, &resources).unwrap_err();
        assert!(error.to_string().contains("requires Own"), "{error}");
    }

    #[wasm_bindgen_test]
    fn supplies_a_valid_placeholder_for_a_failed_import() {
        let JsResult::Poison(value) = default_result(&FunctionType {
            params: Vec::new(),
            result: Some(ValueType::Tuple(vec![ValueType::U32, ValueType::String])),
        })
        .unwrap() else {
            panic!("placeholder unexpectedly threw")
        };
        assert_eq!(
            lift_result(
                value,
                &FunctionType {
                    params: Vec::new(),
                    result: Some(ValueType::Tuple(vec![ValueType::U32, ValueType::String])),
                }
            )
            .unwrap(),
            [Val::Tuple(vec![Val::U32(0), Val::from("")])]
        );
    }

    fn lift_error(value: JsValue, expected: ValueType) -> String {
        let arguments = Array::new();
        arguments.push(&value);
        lift_args(
            &arguments,
            &FunctionType {
                params: vec![expected],
                result: None,
            },
        )
        .unwrap_err()
        .to_string()
    }
}
