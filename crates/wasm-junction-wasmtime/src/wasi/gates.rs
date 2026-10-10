use wasm_junction_core::{
    CallError, CallErrorKind, ChannelDirection, EngineEvent, InvocationId,
    Resource as JunctionResource, ResourceOwnership, Val, Vals, validate_resource_for_invocation,
};
#[cfg(feature = "wasi-p3")]
use wasmtime::component::StreamReader;
#[cfg(feature = "wasi-p3")]
use wasmtime::component::{ComponentType, FutureReader};
use wasmtime::component::{Linker, Resource};
#[cfg(feature = "wasi-p3")]
use wasmtime::{AsContextMut, StoreContextMut};
use wasmtime_wasi::p2::bindings::cli::terminal_input::TerminalInput;
use wasmtime_wasi::p2::bindings::cli::terminal_output::TerminalOutput;
use wasmtime_wasi::p2::bindings::clocks::wall_clock::Datetime;
use wasmtime_wasi::p2::bindings::io::error::HostError;
use wasmtime_wasi::p2::bindings::io::streams::{
    self, HostInputStream, HostOutputStream, StreamError,
};
use wasmtime_wasi::p2::{DynInputStream, DynOutputStream, DynPollable, IoError, StreamResult};

#[cfg(feature = "wasi-p3")]
use super::trampoline::RealConcurrent;
use super::trampoline::{self, Real};
use crate::engine::StoreData;

trait ToVal {
    fn to_val(self) -> Val;
}

trait FromVal: Sized {
    fn from_val(value: Val) -> Result<Self, CallError>;
}

trait WitResource: 'static {
    const INTERFACE: &'static str;
    const NAME: &'static str;
}

fn shape(expected: &str) -> CallError {
    CallError::trap(format!("expected {expected}"))
}

fn scope(value: Val, invocation: InvocationId) -> Val {
    match value {
        Val::Resource(resource) => Val::Resource(match resource.ownership() {
            ResourceOwnership::Own => JunctionResource::__owned_for_invocation(
                resource.interface(),
                resource.name(),
                resource.id(),
                invocation,
            ),
            ResourceOwnership::Borrow => JunctionResource::__borrowed_for_invocation(
                resource.interface(),
                resource.name(),
                resource.id(),
                invocation,
            ),
        }),
        Val::List(values) => Val::List(
            values
                .into_iter()
                .map(|value| scope(value, invocation))
                .collect(),
        ),
        Val::Tuple(values) => Val::Tuple(
            values
                .into_iter()
                .map(|value| scope(value, invocation))
                .collect(),
        ),
        Val::Option(value) => Val::Option(value.map(|value| Box::new(scope(*value, invocation)))),
        Val::Result(result) => Val::Result(match result {
            Ok(value) => Ok(value.map(|value| Box::new(scope(*value, invocation)))),
            Err(value) => Err(value.map(|value| Box::new(scope(*value, invocation)))),
        }),
        Val::Variant { case, value } => Val::Variant {
            case,
            value: value.map(|value| Box::new(scope(*value, invocation))),
        },
        Val::Record(fields) => Val::Record(
            fields
                .into_iter()
                .map(|(name, value)| (name, scope(value, invocation)))
                .collect(),
        ),
        value => value,
    }
}

fn scope_values(values: Vals, invocation: InvocationId) -> Vals {
    values
        .into_iter()
        .map(|value| scope(value, invocation))
        .collect()
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "validators share one fallible function signature in the gate macro"
)]
fn no_resource_validation(_values: &[Val], _store: &mut StoreData) -> Result<(), CallError> {
    Ok(())
}

fn validate_borrowed<T: WitResource>(value: &Val, store: &mut StoreData) -> Result<(), CallError> {
    validate_borrowed_resource::<T>(value, T::INTERFACE, T::NAME, store)
}

fn validate_borrowed_resource<T: 'static>(
    value: &Val,
    interface: &str,
    name: &str,
    store: &mut StoreData,
) -> Result<(), CallError> {
    let Val::Resource(resource) = value else {
        return Err(CallError::refused(format!("expected {name} handle")));
    };
    let invocation = store
        .context
        .invocation_id()
        .ok_or_else(|| CallError::trap("WASI call has no invocation id"))?;
    validate_resource_for_invocation(
        resource,
        interface,
        name,
        ResourceOwnership::Borrow,
        invocation,
    )?;
    store
        .wasi_table()
        .get(&Resource::<T>::new_borrow(resource.id()))
        .map_err(|_| CallError::refused(format!("unknown {name} handle {}", resource.id())))?;
    Ok(())
}

fn validate_owned<T: WitResource>(
    resource: &JunctionResource,
    store: &mut StoreData,
) -> Result<(), CallError> {
    validate_owned_resource::<T>(resource, T::INTERFACE, T::NAME, store)
}

fn validate_owned_resource<T: 'static>(
    resource: &JunctionResource,
    interface: &str,
    name: &str,
    store: &mut StoreData,
) -> Result<(), CallError> {
    let invocation = store
        .context
        .invocation_id()
        .ok_or_else(|| CallError::trap("WASI call has no invocation id"))?;
    validate_resource_for_invocation(
        resource,
        interface,
        name,
        ResourceOwnership::Own,
        invocation,
    )?;
    store
        .wasi_table()
        .get(&Resource::<T>::new_borrow(resource.id()))
        .map_err(|_| CallError::refused(format!("unknown {name} handle {}", resource.id())))?;
    Ok(())
}

fn validate_input_borrow(values: &[Val], store: &mut StoreData) -> Result<(), CallError> {
    let value = values.first().ok_or_else(|| shape("input-stream"))?;
    validate_borrowed::<DynInputStream>(value, store)?;
    validate_handle_contexts(values, store)
}

fn validate_output_borrow(values: &[Val], store: &mut StoreData) -> Result<(), CallError> {
    let value = values.first().ok_or_else(|| shape("output-stream"))?;
    validate_borrowed::<DynOutputStream>(value, store)?;
    validate_handle_contexts(values, store)
}

pub(super) fn add_handle_contexts(values: &mut Vals, store: &StoreData) {
    let contexts = values
        .iter()
        .filter_map(|value| match value {
            Val::Resource(resource) => store.wasi_handle_context(resource.id()).cloned(),
            _ => None,
        })
        .collect::<Vec<_>>();
    values.extend(contexts);
}

pub(super) fn validate_handle_contexts(values: &[Val], store: &StoreData) -> Result<(), CallError> {
    let expected = values
        .iter()
        .filter_map(|value| match value {
            Val::Resource(resource) => store.wasi_handle_context(resource.id()),
            _ => None,
        })
        .collect::<Vec<_>>();
    if expected.is_empty() {
        return Ok(());
    }
    let start = values
        .len()
        .checked_sub(expected.len())
        .ok_or_else(|| CallError::refused("missing WASI handle context"))?;
    (values[start..].iter().eq(expected))
        .then_some(())
        .ok_or_else(|| CallError::refused("WASI handle context does not match"))
}

fn copy_handle_context(store: &mut StoreData, source: u32, target: u32) {
    if let Some(context) = store.wasi_handle_context(source).cloned() {
        store.set_wasi_handle_context(target, context);
    }
}

#[cfg(feature = "wasi-p3")]
#[cfg_attr(not(feature = "wasi-http"), allow(dead_code))]
fn lift_future_plain<T: ComponentType + 'static>(
    store: &mut StoreContextMut<'_, StoreData>,
    future: FutureReader<T>,
) -> wasmtime::Result<Val> {
    let future = future.try_into_future_any(store.as_context_mut())?;
    crate::engine::lift_future(future, store.data_mut()).map(Val::Future)
}

#[cfg(feature = "wasi-p3")]
#[cfg_attr(not(feature = "wasi-http"), allow(dead_code))]
fn lower_future_plain<T: ComponentType + 'static>(
    store: &mut StoreContextMut<'_, StoreData>,
    value: Val,
) -> wasmtime::Result<FutureReader<T>> {
    let Val::Future(future) = value else {
        return Err(wasmtime::Error::new(shape("future")));
    };
    let future = crate::engine::lower_future(&future, store.data_mut())?;
    FutureReader::try_from_future_any(future)
}

#[cfg(all(feature = "wasi-http", feature = "wasi-p3"))]
fn lift_stream_plain(
    store: &mut StoreContextMut<'_, StoreData>,
    stream: StreamReader<u8>,
) -> wasmtime::Result<Val> {
    lift_stream_with_direction_plain(store, stream, ChannelDirection::GuestToHost)
}

#[cfg(feature = "wasi-p3")]
fn lift_stream_with_direction_plain(
    store: &mut StoreContextMut<'_, StoreData>,
    stream: StreamReader<u8>,
    direction: ChannelDirection,
) -> wasmtime::Result<Val> {
    let stream = stream.try_into_stream_any(store.as_context_mut())?;
    crate::streams::lift_stream_with_direction(stream, store.as_context_mut(), direction)
        .map(Val::Stream)
}

#[cfg(all(feature = "wasi-http", feature = "wasi-p3"))]
fn lower_stream_plain(
    store: &mut StoreContextMut<'_, StoreData>,
    value: Val,
) -> wasmtime::Result<StreamReader<u8>> {
    let Val::Stream(stream) = value else {
        return Err(wasmtime::Error::new(shape("stream")));
    };
    let stream = crate::streams::lower_stream(stream, store.as_context_mut())?;
    StreamReader::try_from_stream_any(stream)
}

#[cfg(feature = "wasi-p3")]
fn lower_stream_handoff_plain(
    store: &mut StoreContextMut<'_, StoreData>,
    value: Val,
) -> wasmtime::Result<StreamReader<u8>> {
    let Val::Stream(stream) = value else {
        return Err(wasmtime::Error::new(shape("stream")));
    };
    let stream = crate::streams::lower_stream_with_direction(stream, store.as_context_mut(), None)?;
    StreamReader::try_from_stream_any(stream)
}

#[cfg(all(feature = "wasi-http", feature = "wasi-p3"))]
fn lift_optional_stream_plain(
    store: &mut StoreContextMut<'_, StoreData>,
    stream: Option<StreamReader<u8>>,
) -> wasmtime::Result<Val> {
    stream
        .map(|stream| lift_stream_plain(store, stream))
        .transpose()
        .map(|stream| Val::Option(stream.map(Box::new)))
}

#[cfg(all(feature = "wasi-http", feature = "wasi-p3"))]
fn lower_optional_stream_plain(
    store: &mut StoreContextMut<'_, StoreData>,
    value: Val,
) -> wasmtime::Result<Option<StreamReader<u8>>> {
    let Val::Option(stream) = value else {
        return Err(wasmtime::Error::new(shape("optional stream")));
    };
    stream
        .map(|stream| lower_stream_plain(store, *stream))
        .transpose()
}

fn validate_splice_borrows(values: &[Val], store: &mut StoreData) -> Result<(), CallError> {
    validate_borrowed::<DynOutputStream>(
        values.first().ok_or_else(|| shape("output-stream"))?,
        store,
    )?;
    validate_borrowed::<DynInputStream>(
        values.get(1).ok_or_else(|| shape("input-stream"))?,
        store,
    )?;
    validate_handle_contexts(values, store)
}

fn validate_error_borrow(values: &[Val], store: &mut StoreData) -> Result<(), CallError> {
    validate_borrowed::<IoError>(values.first().ok_or_else(|| shape("error"))?, store)
}

fn open_channel<T>(
    resource: &Resource<T>,
    store: &mut StoreData,
    direction: ChannelDirection,
) -> wasmtime::Result<()> {
    if store.open_wasi_channel(resource.rep()) {
        let invocation = store
            .context
            .invocation_id()
            .ok_or_else(|| wasmtime::Error::msg("WASI call has no invocation id"))?;
        store.imports.emit(EngineEvent::ChannelOpen {
            invocation,
            stream: u64::from(resource.rep()),
            direction,
        });
    }
    Ok(())
}

fn get_stdin(store: &mut StoreData) -> wasmtime::Result<Resource<DynInputStream>> {
    let stream = wasmtime_wasi::p2::bindings::cli::stdin::Host::get_stdin(&mut views::cli(store))?;
    open_channel(&stream, store, ChannelDirection::HostToGuest)?;
    Ok(stream)
}

fn get_stdout(store: &mut StoreData) -> wasmtime::Result<Resource<DynOutputStream>> {
    let stream =
        wasmtime_wasi::p2::bindings::cli::stdout::Host::get_stdout(&mut views::cli(store))?;
    open_channel(&stream, store, ChannelDirection::GuestToHost)?;
    Ok(stream)
}

fn get_stderr(store: &mut StoreData) -> wasmtime::Result<Resource<DynOutputStream>> {
    let stream =
        wasmtime_wasi::p2::bindings::cli::stderr::Host::get_stderr(&mut views::cli(store))?;
    open_channel(&stream, store, ChannelDirection::GuestToHost)?;
    Ok(stream)
}

fn drop_terminal_input(
    store: &mut StoreData,
    resource: Resource<TerminalInput>,
) -> wasmtime::Result<()> {
    wasmtime_wasi::p2::bindings::cli::terminal_input::HostTerminalInput::drop(
        &mut views::cli(store),
        resource,
    )
}

fn drop_terminal_output(
    store: &mut StoreData,
    resource: Resource<TerminalOutput>,
) -> wasmtime::Result<()> {
    wasmtime_wasi::p2::bindings::cli::terminal_output::HostTerminalOutput::drop(
        &mut views::cli(store),
        resource,
    )
}

fn close_channel(store: &mut StoreData, id: u32, direction: Option<ChannelDirection>) {
    let Some(direction) = direction else { return };
    if store.close_wasi_channel(id)
        && let Some(invocation) = store.context.invocation_id()
    {
        store.imports.emit(EngineEvent::ChannelClose {
            invocation,
            stream: u64::from(id),
            direction,
        });
    }
}

fn validate_pollable_borrows(values: &[Val], store: &mut StoreData) -> Result<(), CallError> {
    let invocation = store
        .context
        .invocation_id()
        .ok_or_else(|| CallError::trap("WASI call has no invocation id"))?;
    let declared = values.first().ok_or_else(|| shape(POLLABLE))?;
    let pollables = match declared {
        Val::List(values) => values.as_slice(),
        value => std::slice::from_ref(value),
    };
    for value in pollables {
        match value {
            Val::Resource(resource) => {
                validate_resource_for_invocation(
                    resource,
                    POLLABLE_INTERFACE,
                    POLLABLE,
                    ResourceOwnership::Borrow,
                    invocation,
                )?;
                store
                    .wasi_table()
                    .get(&Resource::<DynPollable>::new_borrow(resource.id()))
                    .map_err(|_| {
                        CallError::refused(format!("unknown pollable handle {}", resource.id()))
                    })?;
            }
            _ => return Err(shape(POLLABLE)),
        }
    }
    validate_handle_contexts(values, store)
}
impl ToVal for String {
    fn to_val(self) -> Val {
        Val::String(self)
    }
}

impl FromVal for String {
    fn from_val(value: Val) -> Result<Self, CallError> {
        match value {
            Val::String(value) => Ok(value),
            _ => Err(shape("string")),
        }
    }
}

impl ToVal for u64 {
    fn to_val(self) -> Val {
        Val::U64(self)
    }
}

impl FromVal for u64 {
    fn from_val(value: Val) -> Result<Self, CallError> {
        match value {
            Val::U64(value) => Ok(value),
            _ => Err(shape("u64")),
        }
    }
}

#[cfg(feature = "wasi-p3")]
impl ToVal for i64 {
    fn to_val(self) -> Val {
        Val::S64(self)
    }
}

#[cfg(feature = "wasi-p3")]
impl FromVal for i64 {
    fn from_val(value: Val) -> Result<Self, CallError> {
        match value {
            Val::S64(value) => Ok(value),
            _ => Err(shape("s64")),
        }
    }
}

impl ToVal for u8 {
    fn to_val(self) -> Val {
        Val::U8(self)
    }
}

impl FromVal for u8 {
    fn from_val(value: Val) -> Result<Self, CallError> {
        match value {
            Val::U8(value) => Ok(value),
            _ => Err(shape("u8")),
        }
    }
}

impl ToVal for u16 {
    fn to_val(self) -> Val {
        Val::U16(self)
    }
}

impl FromVal for u16 {
    fn from_val(value: Val) -> Result<Self, CallError> {
        match value {
            Val::U16(value) => Ok(value),
            _ => Err(shape("u16")),
        }
    }
}

impl ToVal for bool {
    fn to_val(self) -> Val {
        Val::Bool(self)
    }
}

impl FromVal for bool {
    fn from_val(value: Val) -> Result<Self, CallError> {
        match value {
            Val::Bool(value) => Ok(value),
            _ => Err(shape("bool")),
        }
    }
}

impl ToVal for u32 {
    fn to_val(self) -> Val {
        Val::U32(self)
    }
}

impl FromVal for u32 {
    fn from_val(value: Val) -> Result<Self, CallError> {
        match value {
            Val::U32(value) => Ok(value),
            _ => Err(shape("u32")),
        }
    }
}

const POLLABLE_INTERFACE: &str = "wasi:io/poll@0.2.12";
const POLLABLE: &str = "pollable";
const STREAMS_INTERFACE: &str = "wasi:io/streams@0.2.12";
const INPUT_STREAM: &str = "input-stream";
const OUTPUT_STREAM: &str = "output-stream";
const ERROR_INTERFACE: &str = "wasi:io/error@0.2.12";
const ERROR: &str = "error";
const TERMINAL_INPUT_INTERFACE: &str = "wasi:cli/terminal-input@0.2.12";
const TERMINAL_INPUT: &str = "terminal-input";
const TERMINAL_OUTPUT_INTERFACE: &str = "wasi:cli/terminal-output@0.2.12";
const TERMINAL_OUTPUT: &str = "terminal-output";

impl WitResource for DynPollable {
    const INTERFACE: &'static str = POLLABLE_INTERFACE;
    const NAME: &'static str = POLLABLE;
}

impl WitResource for DynInputStream {
    const INTERFACE: &'static str = STREAMS_INTERFACE;
    const NAME: &'static str = INPUT_STREAM;
}

impl WitResource for DynOutputStream {
    const INTERFACE: &'static str = STREAMS_INTERFACE;
    const NAME: &'static str = OUTPUT_STREAM;
}

impl WitResource for IoError {
    const INTERFACE: &'static str = ERROR_INTERFACE;
    const NAME: &'static str = ERROR;
}

impl WitResource for TerminalInput {
    const INTERFACE: &'static str = TERMINAL_INPUT_INTERFACE;
    const NAME: &'static str = TERMINAL_INPUT;
}

impl WitResource for TerminalOutput {
    const INTERFACE: &'static str = TERMINAL_OUTPUT_INTERFACE;
    const NAME: &'static str = TERMINAL_OUTPUT;
}

impl<T: WitResource> ToVal for Resource<T> {
    fn to_val(self) -> Val {
        resource_to_val(&self, T::INTERFACE, T::NAME)
    }
}

impl<T: WitResource> FromVal for Resource<T> {
    fn from_val(value: Val) -> Result<Self, CallError> {
        resource_from_val(value, T::INTERFACE, T::NAME)
    }
}

fn resource_to_val<T>(resource: &Resource<T>, interface: &str, name: &str) -> Val {
    Val::Resource(if resource.owned() {
        JunctionResource::owned(interface, name, resource.rep())
    } else {
        JunctionResource::borrowed(interface, name, resource.rep())
    })
}

fn resource_from_val<T>(value: Val, interface: &str, name: &str) -> Result<Resource<T>, CallError> {
    match value {
        Val::Resource(resource) if resource.interface() == interface && resource.name() == name => {
            Ok(match resource.ownership() {
                ResourceOwnership::Own => Resource::new_own(resource.id()),
                ResourceOwnership::Borrow => Resource::new_borrow(resource.id()),
            })
        }
        _ => Err(shape(name)),
    }
}

macro_rules! list_value {
    ($ty:ty) => {
        impl ToVal for Vec<$ty> {
            fn to_val(self) -> Val {
                Val::List(self.into_iter().map(ToVal::to_val).collect())
            }
        }

        impl FromVal for Vec<$ty> {
            fn from_val(value: Val) -> Result<Self, CallError> {
                match value {
                    Val::List(items) => items.into_iter().map(FromVal::from_val).collect(),
                    _ => Err(shape("list")),
                }
            }
        }
    };
}

list_value!(String);
list_value!((String, String));
#[cfg(feature = "wasi-http")]
list_value!((String, Vec<u8>));
#[cfg(feature = "wasi-http")]
list_value!(Vec<u8>);
list_value!(u32);
list_value!(Resource<DynPollable>);

impl ToVal for Vec<u8> {
    fn to_val(self) -> Val {
        Val::Bytes(self)
    }
}

impl FromVal for Vec<u8> {
    fn from_val(value: Val) -> Result<Self, CallError> {
        match value {
            Val::Bytes(bytes) => Ok(bytes),
            _ => Err(shape("bytes")),
        }
    }
}

impl<T: ToVal> ToVal for Option<T> {
    fn to_val(self) -> Val {
        Val::Option(self.map(|value| Box::new(value.to_val())))
    }
}

impl<T: FromVal> FromVal for Option<T> {
    fn from_val(value: Val) -> Result<Self, CallError> {
        match value {
            Val::Option(value) => value.map(|value| T::from_val(*value)).transpose(),
            _ => Err(shape("option")),
        }
    }
}

impl<A: ToVal, B: ToVal> ToVal for (A, B) {
    fn to_val(self) -> Val {
        Val::Tuple(vec![self.0.to_val(), self.1.to_val()])
    }
}

impl<A: FromVal, B: FromVal> FromVal for (A, B) {
    fn from_val(value: Val) -> Result<Self, CallError> {
        let Val::Tuple(fields) = value else {
            return Err(shape("tuple"));
        };
        let [first, second] = <[Val; 2]>::try_from(fields).map_err(|_| shape("pair"))?;
        Ok((A::from_val(first)?, B::from_val(second)?))
    }
}

impl<A: ToVal, B: ToVal, C: ToVal> ToVal for (A, B, C) {
    fn to_val(self) -> Val {
        Val::Tuple(vec![self.0.to_val(), self.1.to_val(), self.2.to_val()])
    }
}

impl<A: FromVal, B: FromVal, C: FromVal> FromVal for (A, B, C) {
    fn from_val(value: Val) -> Result<Self, CallError> {
        let Val::Tuple(fields) = value else {
            return Err(shape("tuple"));
        };
        let [first, second, third] = <[Val; 3]>::try_from(fields).map_err(|_| shape("triple"))?;
        Ok((
            A::from_val(first)?,
            B::from_val(second)?,
            C::from_val(third)?,
        ))
    }
}

impl ToVal for Datetime {
    fn to_val(self) -> Val {
        Val::Record(vec![
            ("seconds".to_owned(), Val::U64(self.seconds)),
            ("nanoseconds".to_owned(), Val::U32(self.nanoseconds)),
        ])
    }
}

impl FromVal for Datetime {
    fn from_val(value: Val) -> Result<Self, CallError> {
        let Val::Record(fields) = value else {
            return Err(shape("datetime"));
        };
        match fields.as_slice() {
            [(seconds, Val::U64(value)), (nanoseconds, Val::U32(nanos))]
                if seconds == "seconds" && nanoseconds == "nanoseconds" =>
            {
                Ok(Self {
                    seconds: *value,
                    nanoseconds: *nanos,
                })
            }
            _ => Err(shape("datetime fields")),
        }
    }
}

#[cfg(feature = "wasi-p3")]
impl ToVal for wasmtime_wasi::p3::bindings::clocks::system_clock::Instant {
    fn to_val(self) -> Val {
        Val::Record(vec![
            ("seconds".to_owned(), self.seconds.to_val()),
            ("nanoseconds".to_owned(), self.nanoseconds.to_val()),
        ])
    }
}

#[cfg(feature = "wasi-p3")]
impl FromVal for wasmtime_wasi::p3::bindings::clocks::system_clock::Instant {
    fn from_val(value: Val) -> Result<Self, CallError> {
        let Val::Record(fields) = value else {
            return Err(shape("instant"));
        };
        let [(_, seconds), (_, nanoseconds)] =
            <[(String, Val); 2]>::try_from(fields).map_err(|_| shape("instant fields"))?;
        Ok(Self {
            seconds: i64::from_val(seconds)?,
            nanoseconds: u32::from_val(nanoseconds)?,
        })
    }
}

impl ToVal for () {
    fn to_val(self) -> Val {
        Val::Tuple(Vec::new())
    }
}

impl FromVal for () {
    fn from_val(_value: Val) -> Result<Self, CallError> {
        Ok(())
    }
}

impl ToVal for Result<(), ()> {
    fn to_val(self) -> Val {
        Val::Result(self.map(|()| None).map_err(|()| None))
    }
}

impl FromVal for Result<(), ()> {
    fn from_val(value: Val) -> Result<Self, CallError> {
        match value {
            Val::Result(Ok(None)) => Ok(Ok(())),
            Val::Result(Err(None)) => Ok(Err(())),
            _ => Err(shape("result")),
        }
    }
}

impl ToVal for StreamError {
    fn to_val(self) -> Val {
        match self {
            Self::Closed => Val::Variant {
                case: "closed".to_owned(),
                value: None,
            },
            Self::LastOperationFailed(error) => Val::Variant {
                case: "last-operation-failed".to_owned(),
                value: Some(Box::new(error.to_val())),
            },
        }
    }
}

impl FromVal for StreamError {
    fn from_val(value: Val) -> Result<Self, CallError> {
        match value {
            Val::Variant { case, value: None } if case == "closed" => Ok(Self::Closed),
            Val::Variant {
                case,
                value: Some(error),
            } if case == "last-operation-failed" => {
                Resource::from_val(*error).map(Self::LastOperationFailed)
            }
            _ => Err(shape("stream-error")),
        }
    }
}

impl<T: ToVal> ToVal for Result<T, StreamError> {
    fn to_val(self) -> Val {
        Val::Result(match self {
            Ok(value) => Ok(Some(Box::new(value.to_val()))),
            Err(error) => Err(Some(Box::new(error.to_val()))),
        })
    }
}

impl<T: FromVal> FromVal for Result<T, StreamError> {
    fn from_val(value: Val) -> Result<Self, CallError> {
        match value {
            Val::Result(Ok(Some(value))) => T::from_val(*value).map(Ok),
            Val::Result(Ok(None)) => T::from_val(Val::Tuple(Vec::new())).map(Ok),
            Val::Result(Err(Some(error))) => StreamError::from_val(*error).map(Err),
            _ => Err(shape("stream result")),
        }
    }
}

fn convert_stream<T>(
    store: &mut StoreData,
    result: StreamResult<T>,
) -> Result<Result<T, StreamError>, CallError> {
    match result {
        Ok(value) => Ok(Ok(value)),
        Err(error) => streams::Host::convert_stream_error(store.wasi_table(), error)
            .map(Err)
            .map_err(|error| CallError::trap(error.to_string())),
    }
}

fn finish<T: FromVal>(outcome: Result<Vals, CallError>) -> wasmtime::Result<T> {
    let values = outcome.map_err(wasmtime::Error::new)?;
    let [value] = <[Val; 1]>::try_from(values).map_err(|_| shape("one result"))?;
    T::from_val(value).map_err(wasmtime::Error::new)
}

fn finish_unit(outcome: Result<Vals, CallError>) -> wasmtime::Result<()> {
    let values = outcome.map_err(wasmtime::Error::new)?;
    if values.is_empty() {
        Ok(())
    } else {
        Err(wasmtime::Error::new(shape("no results")))
    }
}

#[cfg(feature = "wasi-p3")]
#[cfg_attr(not(feature = "wasi-http"), allow(dead_code))]
fn finish_p3_error<T, E>(
    outcome: Result<Vals, CallError>,
    denied: E,
    decode: impl FnOnce(Vals) -> Result<Result<T, E>, CallError>,
) -> wasmtime::Result<Result<T, E>> {
    match outcome {
        Err(error) if error.kind() == CallErrorKind::Refused => Ok(Err(denied)),
        Err(error) => Err(wasmtime::Error::new(error)),
        Ok(values) => decode(values).map_err(wasmtime::Error::new),
    }
}

#[cfg(feature = "wasi-p3")]
#[cfg_attr(not(feature = "wasi-http"), allow(dead_code))]
fn decode_p3_result<T: FromVal, E: FromVal>(values: Vals) -> Result<Result<T, E>, CallError> {
    let [value] = <[Val; 1]>::try_from(values).map_err(|_| shape("one result"))?;
    let Val::Result(result) = value else {
        return Err(shape("result"));
    };
    match result {
        Ok(value) => {
            T::from_val(value.map_or_else(|| Val::Tuple(Vec::new()), |value| *value)).map(Ok)
        }
        Err(value) => {
            E::from_val(value.map_or_else(|| Val::Tuple(Vec::new()), |value| *value)).map(Err)
        }
    }
}

#[cfg(feature = "wasi-p3")]
#[cfg_attr(not(feature = "wasi-http"), allow(dead_code))]
fn p3_result_value<T: ToVal, E: ToVal>(result: Result<T, E>) -> Val {
    Val::Result(match result {
        Ok(value) => Ok(Some(Box::new(value.to_val()))),
        Err(error) => Err(Some(Box::new(error.to_val()))),
    })
}

#[cfg(feature = "wasi-p3")]
#[cfg_attr(not(feature = "wasi-http"), allow(dead_code))]
fn convert_trappable<T, E>(
    result: Result<T, wasmtime_wasi::TrappableError<E>>,
) -> Result<Result<T, E>, CallError>
where
    E: std::error::Error + Send + Sync + 'static,
{
    match result {
        Ok(value) => Ok(Ok(value)),
        Err(error) => match error.downcast() {
            Ok(error) => Ok(Err(error)),
            Err(error) => Err(CallError::trap(error.to_string())),
        },
    }
}

fn finish_stream<T: FromVal>(
    store: &mut StoreData,
    outcome: Result<Vals, CallError>,
) -> wasmtime::Result<Result<T, StreamError>> {
    let outcome = match outcome {
        Ok(values) => Ok(values),
        Err(error) if error.kind() == CallErrorKind::Refused => {
            let error = store
                .wasi_table()
                .push(wasmtime::Error::msg(error.to_string()))?;
            return Ok(Err(StreamError::LastOperationFailed(error)));
        }
        Err(error) => Err(error),
    };
    finish(outcome)
}

macro_rules! gate {
    ($linker:ident, $iface:literal, $name:literal, $view:ident, $method:path,
     plain_result_with[$validate:ident, $denied:expr],
     ($($arg:ident: $ty:ty),*) -> Result<$ok:ty, $error:ty>) => {
        $linker.instance($iface)?.func_wrap_async(
            $name,
            |mut store, ($($arg,)*): ($($ty,)*)| Box::new(async move {
                let invocation = store.data().context.invocation_id()
                    .ok_or_else(|| wasmtime::Error::msg("WASI call has no invocation id"))?;
                let args = scope_values(vec![$($arg.to_val()),*], invocation);
                let real: Real = |mut store, args| Box::pin(async move {
                    $validate(&args, store.data_mut())?;
                    #[allow(unused_mut, unused_variables)]
                    let mut args = args.into_iter();
                    $(let $arg = <$ty>::from_val(
                        args.next().ok_or_else(|| shape("another argument"))?
                    )?;)*
                    let result = $method(&mut views::$view(store.data_mut()) $(, $arg)*);
                    Ok(vec![p3_result_value(convert_trappable(result)?)])
                });
                let outcome = trampoline::gate(&mut store, $iface, $name, args, real).await;
                Ok((finish_p3_error(
                    outcome, $denied, decode_p3_result::<$ok, $error>,
                )?,))
            }),
        )?;
    };
    ($linker:ident, $iface:literal, $name:literal, $view:ident, $method:path,
     plain_result[$denied:expr],
     ($($arg:ident: $ty:ty),*) -> Result<$ok:ty, $error:ty>) => {
        $linker.instance($iface)?.func_wrap_async(
            $name,
            |mut store, ($($arg,)*): ($($ty,)*)| Box::new(async move {
                let invocation = store.data().context.invocation_id()
                    .ok_or_else(|| wasmtime::Error::msg("WASI call has no invocation id"))?;
                let args = scope_values(vec![$($arg.to_val()),*], invocation);
                let real: Real = |mut store, args| Box::pin(async move {
                    #[allow(unused_mut, unused_variables)]
                    let mut args = args.into_iter();
                    $(let $arg = <$ty>::from_val(
                        args.next().ok_or_else(|| shape("another argument"))?
                    )?;)*
                    let result = $method(&mut views::$view(store.data_mut()) $(, $arg)*);
                    Ok(vec![p3_result_value(convert_trappable(result)?)])
                });
                let outcome = trampoline::gate(&mut store, $iface, $name, args, real).await;
                Ok((finish_p3_error(
                    outcome, $denied, decode_p3_result::<$ok, $error>,
                )?,))
            }),
        )?;
    };
    ($linker:ident, $iface:literal, $name:literal, $method:path,
     concurrent_result[$data:ty, $getter:path, $denied:expr],
     ($($arg:ident: $ty:ty),*) -> Result<$ok:ty, $error:ty>) => {
        $linker.instance($iface)?.func_wrap_concurrent(
            $name,
            |accessor, ($($arg,)*): ($($ty,)*)| Box::pin(async move {
                let invocation = accessor.with(|mut access| access.get().context.invocation_id())
                    .ok_or_else(|| wasmtime::Error::msg("WASI call has no invocation id"))?;
                let args = scope_values(vec![$($arg.to_val()),*], invocation);
                let real: RealConcurrent = |accessor, args| Box::pin(async move {
                    #[allow(unused_mut, unused_variables)]
                    let mut args = args.into_iter();
                    $(let $arg = <$ty>::from_val(
                        args.next().ok_or_else(|| shape("another argument"))?
                    )?;)*
                    let view = accessor.with_getter::<$data>($getter);
                    let value = $method(&view $(, $arg)*).await
                        .map_err(|error| CallError::trap(error.to_string()))?;
                    Ok(vec![p3_result_value(value)])
                });
                let outcome = trampoline::gate_concurrent(
                    accessor, $iface, $name, args, real,
                ).await;
                Ok((finish_p3_error(outcome, $denied, decode_p3_result::<$ok, $error>)?,))
            }),
        )?;
    };
    ($linker:ident, $iface:literal, $name:literal, $method:path,
     concurrent[$data:ty, $getter:path], ($($arg:ident: $ty:ty),*) -> ()) => {
        $linker.instance($iface)?.func_wrap_concurrent(
            $name,
            |accessor, ($($arg,)*): ($($ty,)*)| Box::pin(async move {
                let invocation = accessor.with(|mut access| access.get().context.invocation_id())
                    .ok_or_else(|| wasmtime::Error::msg("WASI call has no invocation id"))?;
                let args = scope_values(vec![$($arg.to_val()),*], invocation);
                let real: RealConcurrent = |accessor, args| Box::pin(async move {
                    #[allow(unused_mut, unused_variables)]
                    let mut args = args.into_iter();
                    $(let $arg = <$ty>::from_val(
                        args.next().ok_or_else(|| shape("another argument"))?
                    )?;)*
                    let view = accessor.with_getter::<$data>($getter);
                    $method(&view $(, $arg)*).await
                        .map_err(|error| CallError::trap(error.to_string()))?;
                    Ok(Vec::new())
                });
                let outcome = trampoline::gate_concurrent(
                    accessor, $iface, $name, args, real,
                ).await;
                finish_unit(outcome)?;
                Ok(())
            }),
        )?;
    };
    ($linker:ident, $iface:literal, $name:literal, $view:ident, $method:path, plain,
     ($($arg:ident: $ty:ty),*) -> ()) => {
        gate!(@define $linker, $iface, $name, $view, $method, no_resource_validation, ,
            ($($arg: $ty),*) -> (), unit);
    };
    ($linker:ident, $iface:literal, $name:literal, $view:ident, $method:path,
     plain_with[$validate:ident], $signature:tt -> $ok:ty) => {
        gate!(@define $linker, $iface, $name, $view, $method, $validate, ,
            $signature -> $ok, one);
    };
    ($linker:ident, $iface:literal, $name:literal, $view:ident, $method:path, plain,
     $signature:tt -> $ok:ty) => {
        gate!(@define $linker, $iface, $name, $view, $method, no_resource_validation, , $signature -> $ok, one);
    };
    ($linker:ident, $iface:literal, $name:literal, $view:ident, $method:path, resource,
     $signature:tt -> $ok:ty) => {
        gate!(@define $linker, $iface, $name, $view, $method, no_resource_validation, , $signature -> $ok, one);
    };
    ($linker:ident, $iface:literal, $name:literal, $view:ident, $method:path, borrowed,
     ($($arg:ident: $ty:ty),*) -> ()) => {
        gate!(@define $linker, $iface, $name, $view, $method, validate_pollable_borrows, await,
            ($($arg: $ty),*) -> (), unit);
    };
    ($linker:ident, $iface:literal, $name:literal, $view:ident, $method:path, borrowed,
     $signature:tt -> $ok:ty) => {
        gate!(@define $linker, $iface, $name, $view, $method, validate_pollable_borrows, await,
            $signature -> $ok, one);
    };
    ($linker:ident, $iface:literal, $name:literal, $method:path, input,
     $($await:ident)?, $signature:tt -> $ok:ty) => {
        gate!(@stream $linker, $iface, $name, $method, validate_input_borrow, $($await)?,
            $signature -> $ok);
    };
    ($linker:ident, $iface:literal, $name:literal, $method:path, output,
     $($await:ident)?, $signature:tt -> $ok:ty) => {
        gate!(@stream $linker, $iface, $name, $method, validate_output_borrow, $($await)?,
            $signature -> $ok);
    };
    ($linker:ident, $iface:literal, $name:literal, $method:path, splice,
     $($await:ident)?, $signature:tt -> $ok:ty) => {
        gate!(@stream $linker, $iface, $name, $method, validate_splice_borrows, $($await)?,
            $signature -> $ok);
    };
    ($linker:ident, $iface:literal, $name:literal, $view:ident, $method:path, input_plain,
     $signature:tt -> $ok:ty) => {
        gate!(@define $linker, $iface, $name, $view, $method, validate_input_borrow, ,
            $signature -> $ok, one);
    };
    ($linker:ident, $iface:literal, $name:literal, $view:ident, $method:path, output_plain,
     $signature:tt -> $ok:ty) => {
        gate!(@define $linker, $iface, $name, $view, $method, validate_output_borrow, ,
            $signature -> $ok, one);
    };
    ($linker:ident, $iface:literal, $name:literal, $view:ident, $method:path, error_borrowed,
     $signature:tt -> $ok:ty) => {
        gate!(@define $linker, $iface, $name, $view, $method, validate_error_borrow, ,
            $signature -> $ok, one);
    };
    (@define $linker:ident, $iface:literal, $name:literal, $view:ident, $method:path,
     $validate:ident, $($await:ident)?, ($($arg:ident: $ty:ty),*) -> $ok:ty, $shape:ident) => {
        $linker.instance($iface)?.func_wrap_async(
            $name,
            |mut store, ($($arg,)*): ($($ty,)*)| Box::new(async move {
                let invocation = store.data().context.invocation_id()
                    .ok_or_else(|| wasmtime::Error::msg("WASI call has no invocation id"))?;
                let mut args = scope_values(vec![$($arg.to_val()),*], invocation);
                $crate::wasi::gates::add_handle_contexts(&mut args, store.data());
                let real: Real = |mut store, args| Box::pin(async move {
                    $validate(&args, store.data_mut())?;
                    #[allow(unused_mut, unused_variables)]
                    let mut args = args.into_iter();
                    $(let $arg = <$ty>::from_val(
                        args.next().ok_or_else(|| shape("another argument"))?
                    )?;)*
                    let value = $method(&mut views::$view(store.data_mut()) $(, $arg)*) $(.$await)?
                        .map_err(|error| CallError::trap(error.to_string()))?;
                    let invocation = store.data().context.invocation_id()
                        .ok_or_else(|| CallError::trap("WASI call has no invocation id"))?;
                    Ok(scope_values(gate!(@values value, $shape), invocation))
                });
                let outcome = trampoline::gate(&mut store, $iface, $name, args, real).await;
                gate!(@return outcome, $ok, $shape)
            }),
        )?;
    };
    (@stream $linker:ident, $iface:literal, $name:literal, $method:path, $validate:ident,
     $($await:ident)?, ($($arg:ident: $ty:ty),*) -> $ok:ty) => {
        $linker.instance($iface)?.func_wrap_async(
            $name,
            |mut store, ($($arg,)*): ($($ty,)*)| Box::new(async move {
                let invocation = store.data().context.invocation_id()
                    .ok_or_else(|| wasmtime::Error::msg("WASI call has no invocation id"))?;
                let mut args = scope_values(vec![$($arg.to_val()),*], invocation);
                add_handle_contexts(&mut args, store.data());
                let real: Real = |mut store, args| Box::pin(async move {
                    $validate(&args, store.data_mut())?;
                    #[allow(unused_mut, unused_variables)]
                    let mut args = args.into_iter();
                    $(let $arg = <$ty>::from_val(
                        args.next().ok_or_else(|| shape("another argument"))?
                    )?;)*
                    let result = $method(views::io(store.data_mut()) $(, $arg)*) $(.$await)?;
                    let result = convert_stream(store.data_mut(), result)?;
                    let invocation = store.data().context.invocation_id()
                        .ok_or_else(|| CallError::trap("WASI call has no invocation id"))?;
                    Ok(scope_values(vec![result.to_val()], invocation))
                });
                let outcome = trampoline::gate(&mut store, $iface, $name, args, real).await;
                Ok((finish_stream::<$ok>(store.data_mut(), outcome)?,))
            }),
        )?;
    };
    (@values $value:ident, one) => { vec![$value.to_val()] };
    (@values $value:ident, unit) => {{ let _ = $value; Vec::new() }};
    (@return $outcome:ident, $ok:ty, one) => { Ok((finish::<$ok>($outcome)?,)) };
    (@return $outcome:ident, $ok:ty, unit) => {{ finish_unit($outcome)?; Ok(()) }};
}

macro_rules! gate_drop {
    ($linker:ident, $iface:ident, $name:ident, $drop:literal, $ty:ty, $view:ident, $direction:expr,
     $method:path $(, $await:ident)?) => {
        $linker.instance($iface)?.resource_async(
            $name,
            wasmtime::component::ResourceType::host::<$ty>(),
            |mut store, id| Box::new(async move {
                let invocation = store.data().context.invocation_id()
                    .ok_or_else(|| wasmtime::Error::msg("WASI drop has no invocation id"))?;
                let resource = JunctionResource::__owned_for_invocation(
                    $iface, $name, id, invocation,
                );
                // ResourceDrop is a pre-drop lifecycle event; ChannelClose only follows success.
                store.data().imports.emit(EngineEvent::ResourceDrop {
                    invocation,
                    resource: resource.clone(),
                });
                let real: Real = |mut store, args| Box::pin(async move {
                    validate_handle_contexts(&args, store.data())?;
                    let Some(Val::Resource(resource)) = args.first()
                    else {
                        return Err(shape($name));
                    };
                    validate_owned::<$ty>(resource, store.data_mut())?;
                    $method(views::$view(store.data_mut()), Resource::<$ty>::new_own(resource.id()))
                        $(.$await)?
                        .map_err(|error| CallError::trap(error.to_string()))?;
                    Ok(Vec::new())
                });
                let mut args = vec![Val::Resource(resource)];
                add_handle_contexts(&mut args, store.data());
                let values = trampoline::gate(
                    &mut store,
                    $iface,
                    $drop,
                    args,
                    real,
                )
                .await
                .map_err(wasmtime::Error::new)?;
                if !values.is_empty() {
                    return Err(wasmtime::Error::new(shape("no results")));
                }
                close_channel(store.data_mut(), id, $direction);
                store.data_mut().remove_wasi_handle_context(id);
                Ok(())
            }),
        )?;
    };
}

macro_rules! flags_value {
    ($ty:ty { $($flag:ident => $name:literal),+ $(,)? }) => {
        impl ToVal for $ty {
            fn to_val(self) -> Val {
                Val::Flags(vec![$($name.to_owned(),)+].into_iter().zip([
                    $(self.contains(Self::$flag),)+
                ]).filter_map(|(name, set)| set.then_some(name)).collect())
            }
        }

        impl FromVal for $ty {
            fn from_val(value: Val) -> Result<Self, CallError> {
                let Val::Flags(values) = value else { return Err(shape("flags")); };
                let mut flags = Self::empty();
                for value in values {
                    match value.as_str() {
                        $($name => flags |= Self::$flag,)+
                        _ => return Err(shape(stringify!($ty))),
                    }
                }
                Ok(flags)
            }
        }
    };
}

macro_rules! enum_value {
    ($ty:ty { $($variant:ident => $name:literal),+ $(,)? }) => {
        impl ToVal for $ty {
            fn to_val(self) -> Val {
                Val::Enum(match self { $(Self::$variant => $name),+ }.to_owned())
            }
        }

        impl FromVal for $ty {
            fn from_val(value: Val) -> Result<Self, CallError> {
                let Val::Enum(value) = value else { return Err(shape("enum")); };
                match value.as_str() {
                    $($name => Ok(Self::$variant),)+
                    _ => Err(shape(stringify!($ty))),
                }
            }
        }
    };
}

#[cfg(all(feature = "wasi-http", feature = "wasi-p3"))]
macro_rules! gate_concurrent_drop {
    ($linker:ident, $iface:ident, $name:ident, $drop:literal, $ty:ty, $method:path) => {
        $linker.instance($iface)?.resource_concurrent(
            $name,
            wasmtime::component::ResourceType::host::<$ty>(),
            |accessor, id| {
                Box::pin(async move {
                    let (invocation, imports) = accessor.with(|mut access| {
                        let store = access.get();
                        (store.context.invocation_id(), store.imports.clone())
                    });
                    let invocation = invocation
                        .ok_or_else(|| wasmtime::Error::msg("WASI drop has no invocation id"))?;
                    let resource =
                        JunctionResource::__owned_for_invocation($iface, $name, id, invocation);
                    imports.emit(EngineEvent::ResourceDrop {
                        invocation,
                        resource: resource.clone(),
                    });
                    let real: RealConcurrent = |accessor, args| {
                        Box::pin(async move {
                            let [Val::Resource(resource)] =
                                <[Val; 1]>::try_from(args).map_err(|_| shape($name))?
                            else {
                                return Err(shape($name));
                            };
                            accessor.with(|mut access| {
                                validate_owned::<$ty>(&resource, access.get())
                            })?;
                            let view = accessor.with_getter::<wasmtime_wasi_http::WasiHttp>(
                                wasmtime_wasi_http::WasiHttpView::http,
                            );
                            view.with(|access| {
                                $method(access, Resource::<$ty>::new_own(resource.id()))
                            })
                            .map_err(|error| CallError::trap(error.to_string()))?;
                            Ok(Vec::new())
                        })
                    };
                    let values = trampoline::gate_concurrent(
                        accessor,
                        $iface,
                        $drop,
                        vec![Val::Resource(resource)],
                        real,
                    )
                    .await
                    .map_err(wasmtime::Error::new)?;
                    if values.is_empty() {
                        Ok(())
                    } else {
                        Err(wasmtime::Error::new(shape("no results")))
                    }
                })
            },
        )?;
    };
}

#[cfg(feature = "wasi-p3")]
mod cli;
#[cfg(feature = "wasi-p3")]
mod deferred;
mod filesystem;
#[cfg(feature = "wasi-p3")]
mod filesystem_p3;
#[cfg(all(feature = "wasi-http", feature = "wasi-p3"))]
mod http;
#[cfg(feature = "wasi-http")]
mod http_p2;
#[cfg(feature = "wasi-http")]
mod http_values;
mod sockets;

pub(super) const STATIC_STREAM_INTERFACES: &[&str] = &[
    #[cfg(feature = "wasi-p3")]
    filesystem_p3::INTERFACE,
];

#[cfg(feature = "wasi-http")]
pub(super) fn add_http(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    http_p2::add(linker)?;
    #[cfg(feature = "wasi-p3")]
    http::add(linker)?;
    Ok(())
}

pub(super) fn add_filesystem(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    filesystem::add(linker)
}

pub(super) fn add_sockets(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    sockets::add(linker)
}

pub(super) fn add_environment(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    gate!(linker, "wasi:cli/environment@0.2.12", "get-environment", cli,
        wasmtime_wasi::p2::bindings::cli::environment::Host::get_environment,
        plain, () -> Vec<(String, String)>);
    gate!(linker, "wasi:cli/environment@0.2.12", "get-arguments", cli,
        wasmtime_wasi::p2::bindings::cli::environment::Host::get_arguments,
        plain, () -> Vec<String>);
    gate!(linker, "wasi:cli/environment@0.2.12", "initial-cwd", cli,
        wasmtime_wasi::p2::bindings::cli::environment::Host::initial_cwd,
        plain, () -> Option<String>);
    Ok(())
}

pub(super) fn add_stdio(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    gate!(linker, "wasi:cli/stdin@0.2.12", "get-stdin", store,
        get_stdin, resource, () -> Resource<DynInputStream>);
    gate!(linker, "wasi:cli/stdout@0.2.12", "get-stdout", store,
        get_stdout, resource, () -> Resource<DynOutputStream>);
    gate!(linker, "wasi:cli/stderr@0.2.12", "get-stderr", store,
        get_stderr, resource, () -> Resource<DynOutputStream>);
    Ok(())
}

pub(super) fn add_exit(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    gate!(linker, "wasi:cli/exit@0.2.12", "exit", cli,
        wasmtime_wasi::p2::bindings::cli::exit::Host::exit,
        plain, (status: Result<(), ()>) -> ());
    gate!(linker, "wasi:cli/exit@0.2.12", "exit-with-code", cli,
        wasmtime_wasi::p2::bindings::cli::exit::Host::exit_with_code,
        plain, (status_code: u8) -> ());
    Ok(())
}

pub(super) fn add_terminal(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    gate_drop!(
        linker,
        TERMINAL_INPUT_INTERFACE,
        TERMINAL_INPUT,
        "[drop]terminal-input",
        TerminalInput,
        store,
        None,
        drop_terminal_input
    );
    gate_drop!(
        linker,
        TERMINAL_OUTPUT_INTERFACE,
        TERMINAL_OUTPUT,
        "[drop]terminal-output",
        TerminalOutput,
        store,
        None,
        drop_terminal_output
    );
    gate!(linker, "wasi:cli/terminal-stdin@0.2.12", "get-terminal-stdin", cli,
        wasmtime_wasi::p2::bindings::cli::terminal_stdin::Host::get_terminal_stdin,
        resource, () -> Option<Resource<TerminalInput>>);
    gate!(linker, "wasi:cli/terminal-stdout@0.2.12", "get-terminal-stdout", cli,
        wasmtime_wasi::p2::bindings::cli::terminal_stdout::Host::get_terminal_stdout,
        resource, () -> Option<Resource<TerminalOutput>>);
    gate!(linker, "wasi:cli/terminal-stderr@0.2.12", "get-terminal-stderr", cli,
        wasmtime_wasi::p2::bindings::cli::terminal_stderr::Host::get_terminal_stderr,
        resource, () -> Option<Resource<TerminalOutput>>);
    Ok(())
}

pub(super) fn add_wall_clock(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    gate!(linker, "wasi:clocks/wall-clock@0.2.12", "now", clocks,
        wasmtime_wasi::p2::bindings::clocks::wall_clock::Host::now,
        plain, () -> Datetime);
    gate!(linker, "wasi:clocks/wall-clock@0.2.12", "resolution", clocks,
        wasmtime_wasi::p2::bindings::clocks::wall_clock::Host::resolution,
        plain, () -> Datetime);
    Ok(())
}

pub(super) fn add_monotonic_clock(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    gate!(linker, "wasi:clocks/monotonic-clock@0.2.12", "now", clocks,
        wasmtime_wasi::p2::bindings::clocks::monotonic_clock::Host::now,
        plain, () -> u64);
    gate!(linker, "wasi:clocks/monotonic-clock@0.2.12", "subscribe-instant", clocks,
        wasmtime_wasi::p2::bindings::clocks::monotonic_clock::Host::subscribe_instant,
        resource, (when: u64) -> Resource<DynPollable>);
    gate!(linker, "wasi:clocks/monotonic-clock@0.2.12", "subscribe-duration", clocks,
        wasmtime_wasi::p2::bindings::clocks::monotonic_clock::Host::subscribe_duration,
        resource, (duration: u64) -> Resource<DynPollable>);
    gate!(linker, "wasi:clocks/monotonic-clock@0.2.12", "resolution", clocks,
        wasmtime_wasi::p2::bindings::clocks::monotonic_clock::Host::resolution,
        plain, () -> u64);
    Ok(())
}

pub(super) fn add_random(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    gate!(linker, "wasi:random/random@0.2.12", "get-random-bytes", random,
        wasmtime_wasi::p2::bindings::random::random::Host::get_random_bytes,
        plain, (len: u64) -> Vec<u8>);
    gate!(linker, "wasi:random/random@0.2.12", "get-random-u64", random,
        wasmtime_wasi::p2::bindings::random::random::Host::get_random_u64,
        plain, () -> u64);
    gate!(linker, "wasi:random/insecure@0.2.12", "get-insecure-random-bytes", random,
        wasmtime_wasi::p2::bindings::random::insecure::Host::get_insecure_random_bytes,
        plain, (len: u64) -> Vec<u8>);
    gate!(linker, "wasi:random/insecure@0.2.12", "get-insecure-random-u64", random,
        wasmtime_wasi::p2::bindings::random::insecure::Host::get_insecure_random_u64,
        plain, () -> u64);
    gate!(linker, "wasi:random/insecure-seed@0.2.12", "insecure-seed", random,
        wasmtime_wasi::p2::bindings::random::insecure_seed::Host::insecure_seed,
        plain, () -> (u64, u64));
    Ok(())
}

#[cfg(feature = "wasi-p3")]
pub(super) fn add_p3(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    use wasmtime_wasi::clocks::{WasiClocks, WasiClocksView};
    use wasmtime_wasi::p3::bindings::clocks::{monotonic_clock, system_clock};
    use wasmtime_wasi::p3::bindings::random::{insecure, insecure_seed, random};

    cli::add(linker)?;
    linker.instance("wasi:clocks/types@0.3.0")?;
    gate!(linker, "wasi:clocks/monotonic-clock@0.3.0", "now", clocks,
        monotonic_clock::Host::now, plain, () -> u64);
    gate!(linker, "wasi:clocks/monotonic-clock@0.3.0", "get-resolution", clocks,
        monotonic_clock::Host::get_resolution, plain, () -> u64);
    gate!(linker, "wasi:clocks/monotonic-clock@0.3.0", "wait-until",
        monotonic_clock::HostWithStore::wait_until,
        concurrent[WasiClocks, WasiClocksView::clocks], (when: u64) -> ());
    gate!(linker, "wasi:clocks/monotonic-clock@0.3.0", "wait-for",
        monotonic_clock::HostWithStore::wait_for,
        concurrent[WasiClocks, WasiClocksView::clocks], (duration: u64) -> ());
    gate!(linker, "wasi:clocks/system-clock@0.3.0", "now", clocks,
        system_clock::Host::now, plain,
        () -> wasmtime_wasi::p3::bindings::clocks::system_clock::Instant);
    gate!(linker, "wasi:clocks/system-clock@0.3.0", "get-resolution", clocks,
        system_clock::Host::get_resolution, plain, () -> u64);
    gate!(linker, "wasi:random/random@0.3.0", "get-random-bytes", random,
        random::Host::get_random_bytes, plain, (len: u64) -> Vec<u8>);
    gate!(linker, "wasi:random/random@0.3.0", "get-random-u64", random,
        random::Host::get_random_u64, plain, () -> u64);
    gate!(linker, "wasi:random/insecure@0.3.0", "get-insecure-random-bytes", random,
        insecure::Host::get_insecure_random_bytes, plain, (len: u64) -> Vec<u8>);
    gate!(linker, "wasi:random/insecure@0.3.0", "get-insecure-random-u64", random,
        insecure::Host::get_insecure_random_u64, plain, () -> u64);
    gate!(linker, "wasi:random/insecure-seed@0.3.0", "get-insecure-seed", random,
        insecure_seed::Host::get_insecure_seed, plain, () -> (u64, u64));
    Ok(())
}

pub(super) fn add_streams(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    gate_drop!(
        linker,
        STREAMS_INTERFACE,
        INPUT_STREAM,
        "[drop]input-stream",
        DynInputStream,
        io,
        Some(ChannelDirection::HostToGuest),
        HostInputStream::drop,
        await
    );
    gate_drop!(
        linker,
        STREAMS_INTERFACE,
        OUTPUT_STREAM,
        "[drop]output-stream",
        DynOutputStream,
        io,
        Some(ChannelDirection::GuestToHost),
        HostOutputStream::drop,
        await
    );
    gate!(linker, "wasi:io/streams@0.2.12", "[method]input-stream.read",
        HostInputStream::read, input, , (stream: Resource<DynInputStream>, len: u64) -> Vec<u8>);
    gate!(linker, "wasi:io/streams@0.2.12", "[method]input-stream.blocking-read",
        HostInputStream::blocking_read, input, await,
        (stream: Resource<DynInputStream>, len: u64) -> Vec<u8>);
    gate!(linker, "wasi:io/streams@0.2.12", "[method]input-stream.skip",
        HostInputStream::skip, input, , (stream: Resource<DynInputStream>, len: u64) -> u64);
    gate!(linker, "wasi:io/streams@0.2.12", "[method]input-stream.blocking-skip",
        HostInputStream::blocking_skip, input, await,
        (stream: Resource<DynInputStream>, len: u64) -> u64);
    gate!(linker, "wasi:io/streams@0.2.12", "[method]input-stream.subscribe", io,
        HostInputStream::subscribe, input_plain,
        (stream: Resource<DynInputStream>) -> Resource<DynPollable>);
    gate!(linker, "wasi:io/streams@0.2.12", "[method]output-stream.check-write",
        HostOutputStream::check_write, output, , (stream: Resource<DynOutputStream>) -> u64);
    gate!(linker, "wasi:io/streams@0.2.12", "[method]output-stream.write",
        HostOutputStream::write, output, ,
        (stream: Resource<DynOutputStream>, bytes: Vec<u8>) -> ());
    gate!(linker, "wasi:io/streams@0.2.12", "[method]output-stream.blocking-write-and-flush",
        HostOutputStream::blocking_write_and_flush, output, await,
        (stream: Resource<DynOutputStream>, bytes: Vec<u8>) -> ());
    gate!(linker, "wasi:io/streams@0.2.12", "[method]output-stream.flush",
        HostOutputStream::flush, output, , (stream: Resource<DynOutputStream>) -> ());
    gate!(linker, "wasi:io/streams@0.2.12", "[method]output-stream.blocking-flush",
        HostOutputStream::blocking_flush, output, await, (stream: Resource<DynOutputStream>) -> ());
    gate!(linker, "wasi:io/streams@0.2.12", "[method]output-stream.subscribe", io,
        HostOutputStream::subscribe, output_plain,
        (stream: Resource<DynOutputStream>) -> Resource<DynPollable>);
    gate!(linker, "wasi:io/streams@0.2.12", "[method]output-stream.write-zeroes",
        HostOutputStream::write_zeroes, output, , (stream: Resource<DynOutputStream>, len: u64) -> ());
    gate!(linker, "wasi:io/streams@0.2.12", "[method]output-stream.blocking-write-zeroes-and-flush",
        HostOutputStream::blocking_write_zeroes_and_flush, output, await,
        (stream: Resource<DynOutputStream>, len: u64) -> ());
    gate!(linker, "wasi:io/streams@0.2.12", "[method]output-stream.splice",
        HostOutputStream::splice, splice, ,
        (stream: Resource<DynOutputStream>, input: Resource<DynInputStream>, len: u64) -> u64);
    gate!(linker, "wasi:io/streams@0.2.12", "[method]output-stream.blocking-splice",
        HostOutputStream::blocking_splice, splice, await,
        (stream: Resource<DynOutputStream>, input: Resource<DynInputStream>, len: u64) -> u64);
    Ok(())
}

pub(super) fn add_error(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    gate_drop!(
        linker,
        ERROR_INTERFACE,
        ERROR,
        "[drop]error",
        IoError,
        io,
        None,
        HostError::drop
    );
    gate!(linker, "wasi:io/error@0.2.12", "[method]error.to-debug-string", io,
        HostError::to_debug_string, error_borrowed, (error: Resource<IoError>) -> String);
    Ok(())
}

pub(super) fn add_poll(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    gate_drop!(
        linker,
        POLLABLE_INTERFACE,
        POLLABLE,
        "[drop]pollable",
        DynPollable,
        io,
        None,
        wasmtime_wasi::p2::bindings::io::poll::HostPollable::drop
    );
    gate!(linker, "wasi:io/poll@0.2.12", "[method]pollable.ready", io,
        wasmtime_wasi::p2::bindings::io::poll::HostPollable::ready,
        borrowed, (pollable: Resource<DynPollable>) -> bool);
    gate!(linker, "wasi:io/poll@0.2.12", "[method]pollable.block", io,
        wasmtime_wasi::p2::bindings::io::poll::HostPollable::block,
        borrowed, (pollable: Resource<DynPollable>) -> ());
    gate!(linker, "wasi:io/poll@0.2.12", "poll", io,
        wasmtime_wasi::p2::bindings::io::poll::Host::poll,
        borrowed, (pollables: Vec<Resource<DynPollable>>) -> Vec<u32>);
    Ok(())
}

mod views {
    use wasmtime_wasi::cli::WasiCliView;
    use wasmtime_wasi::clocks::WasiClocksView;
    use wasmtime_wasi::filesystem::WasiFilesystemView;
    use wasmtime_wasi::random::WasiRandomView;
    use wasmtime_wasi::sockets::WasiSocketsView;

    use crate::engine::StoreData;

    pub(super) fn cli(store: &mut StoreData) -> wasmtime_wasi::cli::WasiCliCtxView<'_> {
        store.cli()
    }

    pub(super) fn clocks(store: &mut StoreData) -> wasmtime_wasi::clocks::WasiClocksCtxView<'_> {
        store.clocks()
    }

    pub(super) fn io(store: &mut StoreData) -> &mut wasmtime::component::ResourceTable {
        store.wasi_table()
    }

    pub(super) fn filesystem(
        store: &mut StoreData,
    ) -> wasmtime_wasi::filesystem::WasiFilesystemCtxView<'_> {
        store.filesystem()
    }

    pub(super) fn random(store: &mut StoreData) -> &mut wasmtime_wasi::random::WasiRandomCtx {
        store.random()
    }

    pub(super) fn sockets(store: &mut StoreData) -> wasmtime_wasi::sockets::WasiSocketsCtxView<'_> {
        store.sockets()
    }

    pub(super) const fn store(store: &mut StoreData) -> &mut StoreData {
        store
    }

    #[cfg(feature = "wasi-http")]
    pub(super) fn http(store: &mut StoreData) -> wasmtime_wasi_http::WasiHttpCtxView<'_> {
        wasmtime_wasi_http::WasiHttpView::http(store)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "wasi-p3")]
    async fn test_result_gate(
        _store: &wasmtime::component::Accessor<StoreData, wasmtime::component::HasSelf<StoreData>>,
        value: u64,
    ) -> wasmtime::Result<Result<u64, u64>> {
        Ok(Ok(value))
    }

    #[cfg(feature = "wasi-p3")]
    const fn store_data(store: &mut StoreData) -> &mut StoreData {
        store
    }

    #[cfg(all(feature = "wasi-http", feature = "wasi-p3"))]
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the mock matches the trappable host method signature"
    )]
    fn test_plain_result(
        _store: &mut StoreData,
        value: u64,
    ) -> Result<
        u64,
        wasmtime_wasi::TrappableError<wasmtime_wasi_http::p3::bindings::http::types::HeaderError>,
    > {
        Ok(value)
    }

    #[test]
    fn stream_errors_round_trip_through_middleware_values() {
        let closed = StreamError::from_val(StreamError::Closed.to_val()).unwrap();
        assert!(matches!(closed, StreamError::Closed));

        let encoded = StreamError::LastOperationFailed(Resource::<IoError>::new_own(17)).to_val();
        let decoded = StreamError::from_val(encoded).unwrap();
        let StreamError::LastOperationFailed(error) = decoded else {
            panic!("last-operation-failed changed cases");
        };
        assert!(error.owned());
        assert_eq!(error.rep(), 17);
    }

    #[test]
    fn nested_resources_receive_invocation_provenance() {
        let invocation = InvocationId::__from_counter(9);
        let value = scope(
            Val::Result(Ok(Some(Box::new(Val::Resource(JunctionResource::owned(
                "test:api/types",
                "item",
                4,
            )))))),
            invocation,
        );
        let Val::Result(Ok(Some(value))) = value else {
            panic!("result shape changed");
        };
        let Val::Resource(resource) = *value else {
            panic!("resource shape changed");
        };
        assert_eq!(resource.invocation_id(), Some(invocation));
    }

    #[test]
    #[cfg(feature = "wasi-p3")]
    fn p3_system_clock_instants_round_trip_through_middleware_values() {
        use wasmtime_wasi::p3::bindings::clocks::system_clock::Instant;

        let instant = Instant {
            seconds: -1,
            nanoseconds: 999_999_999,
        };
        let decoded = Instant::from_val(instant.to_val()).unwrap();
        assert_eq!(decoded.seconds, -1);
        assert_eq!(decoded.nanoseconds, 999_999_999);
    }

    #[test]
    #[cfg(all(feature = "wasi-http", feature = "wasi-p3"))]
    fn http_scalar_and_header_values_round_trip() {
        assert_eq!(u16::from_val(418_u16.to_val()).unwrap(), 418);
        let headers = vec![("x-test".to_owned(), b"value".to_vec())];
        assert_eq!(
            Vec::<(String, Vec<u8>)>::from_val(headers.clone().to_val()).unwrap(),
            headers
        );
    }

    #[test]
    #[cfg(feature = "wasi-p3")]
    fn p3_refusals_map_to_access_and_other_failures_trap() {
        use wasmtime_wasi::p3::bindings::filesystem::types::ErrorCode;

        let refused: Result<(), ErrorCode> = finish_p3_error(
            Err(CallError::refused("denied")),
            ErrorCode::Access,
            |_| unreachable!(),
        )
        .unwrap();
        assert!(matches!(refused, Err(ErrorCode::Access)));

        let trapped = finish_p3_error::<(), _>(
            Err(CallError::trap("broken")),
            ErrorCode::Access,
            |_| unreachable!(),
        )
        .unwrap_err();
        assert!(trapped.to_string().contains("broken"));
    }

    #[test]
    #[cfg(feature = "wasi-p3")]
    fn concurrent_p3_error_gates_register() -> wasmtime::Result<()> {
        let mut config = wasmtime::Config::new();
        config
            .wasm_component_model_async(true)
            .concurrency_support(true);
        let engine = wasmtime::Engine::new(&config).unwrap();
        let mut linker = Linker::<StoreData>::new(&engine);
        gate!(linker, "test:p3/error@0.1.0", "probe", test_result_gate,
            concurrent_result[wasmtime::component::HasSelf<StoreData>, store_data, 403_u64],
            (value: u64) -> Result<u64, u64>);
        Ok(())
    }

    #[test]
    #[cfg(all(feature = "wasi-http", feature = "wasi-p3"))]
    fn plain_p3_error_gates_register() -> wasmtime::Result<()> {
        use wasmtime_wasi_http::p3::bindings::http::types::HeaderError;

        let engine = wasmtime::Engine::default();
        let mut linker = Linker::<StoreData>::new(&engine);
        gate!(linker, "test:p3/error@0.1.0", "probe", store, test_plain_result,
            plain_result[HeaderError::Forbidden],
            (value: u64) -> Result<u64, HeaderError>);
        Ok(())
    }
}
