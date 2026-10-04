use super::super::{FromVal, ToVal, shape};
use wasm_junction_core::{
    CallError, CallErrorKind, Resource as JunctionResource, ResourceOwnership, Val, Vals,
};
use wasmtime::component::Resource;

use crate::engine::StoreData;
use wasmtime_wasi_http::p2::bindings::http::types::{HeaderError, Method, Scheme};

pub(super) trait HttpResource: 'static {
    const INTERFACE: &'static str;
    const NAME: &'static str;
}

pub(super) trait ToHttpVal {
    fn to_http_val(self) -> Val;
}

pub(super) trait FromHttpVal: Sized {
    fn from_http_val(value: Val) -> Result<Self, CallError>;
}

macro_rules! through_core_codec {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl ToHttpVal for $ty {
                fn to_http_val(self) -> Val { self.to_val() }
            }

            impl FromHttpVal for $ty {
                fn from_http_val(value: Val) -> Result<Self, CallError> {
                    Self::from_val(value)
                }
            }
        )+
    };
}

through_core_codec!(
    String,
    bool,
    u8,
    u16,
    u32,
    u64,
    Vec<u8>,
    Vec<Vec<u8>>,
    Vec<(String, Vec<u8>)>,
    (),
    Method,
    Scheme,
    HeaderError,
);

impl<T: ToHttpVal> ToHttpVal for Option<T> {
    fn to_http_val(self) -> Val {
        Val::Option(self.map(|value| Box::new(value.to_http_val())))
    }
}

impl<T: FromHttpVal> FromHttpVal for Option<T> {
    fn from_http_val(value: Val) -> Result<Self, CallError> {
        let Val::Option(value) = value else {
            return Err(shape("option"));
        };
        value.map(|value| T::from_http_val(*value)).transpose()
    }
}

impl<T: ToHttpVal, E: ToHttpVal> ToHttpVal for Result<T, E> {
    fn to_http_val(self) -> Val {
        Val::Result(match self {
            Ok(value) => Ok(Some(Box::new(value.to_http_val()))),
            Err(error) => Err(Some(Box::new(error.to_http_val()))),
        })
    }
}

impl<T: FromHttpVal, E: FromHttpVal> FromHttpVal for Result<T, E> {
    fn from_http_val(value: Val) -> Result<Self, CallError> {
        let Val::Result(result) = value else {
            return Err(shape("result"));
        };
        match result {
            Ok(Some(value)) => T::from_http_val(*value).map(Ok),
            Err(Some(value)) => E::from_http_val(*value).map(Err),
            _ => Err(shape("result payload")),
        }
    }
}

impl<T: HttpResource> ToHttpVal for Resource<T> {
    fn to_http_val(self) -> Val {
        let resource = if self.owned() {
            JunctionResource::owned(T::INTERFACE, T::NAME, self.rep())
        } else {
            JunctionResource::borrowed(T::INTERFACE, T::NAME, self.rep())
        };
        Val::Resource(resource)
    }
}

impl<T: HttpResource> FromHttpVal for Resource<T> {
    fn from_http_val(value: Val) -> Result<Self, CallError> {
        let Val::Resource(resource) = value else {
            return Err(shape(T::NAME));
        };
        if resource.interface() != T::INTERFACE || resource.name() != T::NAME {
            return Err(shape(T::NAME));
        }
        Ok(match resource.ownership() {
            ResourceOwnership::Own => Self::new_own(resource.id()),
            ResourceOwnership::Borrow => Self::new_borrow(resource.id()),
        })
    }
}

pub(super) fn validate_borrowed<T: HttpResource>(
    value: &Val,
    store: &mut StoreData,
) -> Result<(), CallError> {
    let Val::Resource(resource) = value else {
        return Err(CallError::refused(format!("expected {} handle", T::NAME)));
    };
    let invocation = store
        .context
        .invocation_id()
        .ok_or_else(|| CallError::trap("WASI call has no invocation id"))?;
    wasm_junction_core::validate_resource_for_invocation(
        resource,
        T::INTERFACE,
        T::NAME,
        ResourceOwnership::Borrow,
        invocation,
    )?;
    store
        .wasi_table()
        .get(&Resource::<T>::new_borrow(resource.id()))
        .map_err(|_| CallError::refused(format!("unknown {} handle {}", T::NAME, resource.id())))?;
    Ok(())
}

pub(super) fn validate_owned<T: HttpResource>(
    resource: &JunctionResource,
    store: &mut StoreData,
) -> Result<(), CallError> {
    let invocation = store
        .context
        .invocation_id()
        .ok_or_else(|| CallError::trap("WASI call has no invocation id"))?;
    wasm_junction_core::validate_resource_for_invocation(
        resource,
        T::INTERFACE,
        T::NAME,
        ResourceOwnership::Own,
        invocation,
    )?;
    store
        .wasi_table()
        .get(&Resource::<T>::new_borrow(resource.id()))
        .map_err(|_| CallError::refused(format!("unknown {} handle {}", T::NAME, resource.id())))?;
    Ok(())
}

pub(super) fn finish_http<T: FromHttpVal>(outcome: Result<Vals, CallError>) -> wasmtime::Result<T> {
    let values = outcome.map_err(wasmtime::Error::new)?;
    let [value] = <[Val; 1]>::try_from(values).map_err(|_| shape("one result"))?;
    T::from_http_val(value).map_err(wasmtime::Error::new)
}

pub(super) fn convert_header<T>(
    result: Result<T, wasmtime_wasi_http::p2::HeaderError>,
) -> Result<Result<T, HeaderError>, CallError> {
    match result {
        Ok(value) => Ok(Ok(value)),
        Err(error) => error
            .downcast()
            .map(Err)
            .map_err(|error| CallError::trap(error.to_string())),
    }
}

pub(super) fn convert_plain<T, E>(
    result: wasmtime::Result<Result<T, E>>,
) -> Result<Result<T, E>, CallError> {
    result.map_err(|error| CallError::trap(error.to_string()))
}

pub(super) fn finish_result<T: FromHttpVal, E: FromHttpVal>(
    outcome: Result<Vals, CallError>,
    denied: E,
) -> wasmtime::Result<Result<T, E>> {
    match outcome {
        Err(error) if error.kind() == CallErrorKind::Refused => Ok(Err(denied)),
        other => finish_http(other),
    }
}

macro_rules! gate_http {
    ($linker:ident, $name:literal, $method:path, $validate:ident,
     ($($arg:ident: $ty:ty),*) -> $ok:ty) => {
        gate_http!(@define $linker, $name, http, $method, $validate,
            ($($arg: $ty),*) -> $ok);
    };
    ($linker:ident, $name:literal, store, $method:path, $validate:ident,
     ($($arg:ident: $ty:ty),*) -> $ok:ty) => {
        gate_http!(@define $linker, $name, store, $method, $validate,
            ($($arg: $ty),*) -> $ok);
    };
    (@define $linker:ident, $name:literal, $view:ident, $method:path, $validate:ident,
     ($($arg:ident: $ty:ty),*) -> $ok:ty) => {
        $linker.instance(TYPES)?.func_wrap_async(
            $name,
            |mut store, ($($arg,)*): ($($ty,)*)| Box::new(async move {
                let invocation = store.data().context.invocation_id()
                    .ok_or_else(|| wasmtime::Error::msg("WASI call has no invocation id"))?;
                let mut args = scope_values(vec![$($arg.to_http_val()),*], invocation);
                add_handle_contexts(&mut args, store.data());
                let real: Real = |mut store, args| Box::pin(async move {
                    $validate(&args, store.data_mut())?;
                    #[allow(unused_mut, unused_variables)]
                    let mut args = args.into_iter();
                    $(let $arg = <$ty>::from_http_val(
                        args.next().ok_or_else(|| shape("another argument"))?
                    )?;)*
                    let value = $method(&mut views::$view(store.data_mut()) $(, $arg)*)
                        .map_err(|error| CallError::trap(error.to_string()))?;
                    let invocation = store.data().context.invocation_id()
                        .ok_or_else(|| CallError::trap("WASI call has no invocation id"))?;
                    Ok(scope_values(vec![value.to_http_val()], invocation))
                });
                let outcome = trampoline::gate(&mut store, TYPES, $name, args, real).await;
                Ok((codec::finish_http::<$ok>(outcome)?,))
            }),
        )?;
    };
}

pub(super) use gate_http;

macro_rules! gate_http_result {
    ($linker:ident, $name:literal, $method:path, $validate:ident, $convert:path, $denied:expr,
     ($($arg:ident: $ty:ty),*) -> Result<$ok:ty, $error:ty>) => {
        gate_http_result!(@define $linker, $name, http, $method, $validate, $convert, $denied,
            ($($arg: $ty),*) -> Result<$ok, $error>);
    };
    ($linker:ident, $name:literal, store, $method:path, $validate:ident, $convert:path, $denied:expr,
     ($($arg:ident: $ty:ty),*) -> Result<$ok:ty, $error:ty>) => {
        gate_http_result!(@define $linker, $name, store, $method, $validate, $convert, $denied,
            ($($arg: $ty),*) -> Result<$ok, $error>);
    };
    (@define $linker:ident, $name:literal, $view:ident, $method:path, $validate:ident,
     $convert:path, $denied:expr,
     ($($arg:ident: $ty:ty),*) -> Result<$ok:ty, $error:ty>) => {
        $linker.instance(TYPES)?.func_wrap_async(
            $name,
            |mut store, ($($arg,)*): ($($ty,)*)| Box::new(async move {
                let invocation = store.data().context.invocation_id()
                    .ok_or_else(|| wasmtime::Error::msg("WASI call has no invocation id"))?;
                let mut args = scope_values(vec![$($arg.to_http_val()),*], invocation);
                add_handle_contexts(&mut args, store.data());
                let real: Real = |mut store, args| Box::pin(async move {
                    $validate(&args, store.data_mut())?;
                    #[allow(unused_mut, unused_variables)]
                    let mut args = args.into_iter();
                    $(let $arg = <$ty>::from_http_val(
                        args.next().ok_or_else(|| shape("another argument"))?
                    )?;)*
                    let result = $convert($method(&mut views::$view(store.data_mut()) $(, $arg)*))?;
                    let invocation = store.data().context.invocation_id()
                        .ok_or_else(|| CallError::trap("WASI call has no invocation id"))?;
                    Ok(scope_values(vec![result.to_http_val()], invocation))
                });
                let outcome = trampoline::gate(&mut store, TYPES, $name, args, real).await;
                Ok((codec::finish_result::<$ok, $error>(outcome, $denied)?,))
            }),
        )?;
    };
}

pub(super) use gate_http_result;

macro_rules! gate_http_drop {
    ($linker:ident, $name:literal, $ty:ty, $method:path) => {
        $linker.instance(TYPES)?.resource_async(
            $name,
            wasmtime::component::ResourceType::host::<$ty>(),
            |mut store, id| {
                Box::new(async move {
                    let invocation =
                        store.data().context.invocation_id().ok_or_else(|| {
                            wasmtime::Error::msg("WASI drop has no invocation id")
                        })?;
                    let resource =
                        JunctionResource::__owned_for_invocation(TYPES, $name, id, invocation);
                    store.data().imports.emit(EngineEvent::ResourceDrop {
                        invocation,
                        resource: resource.clone(),
                    });
                    let real: Real = |mut store, args| {
                        Box::pin(async move {
                            validate_handle_contexts(&args, store.data())?;
                            let Some(Val::Resource(resource)) = args.first() else {
                                return Err(shape($name));
                            };
                            codec::validate_owned::<$ty>(resource, store.data_mut())?;
                            $method(
                                &mut views::http(store.data_mut()),
                                Resource::<$ty>::new_own(resource.id()),
                            )
                            .map_err(|error| CallError::trap(error.to_string()))?;
                            Ok(Vec::new())
                        })
                    };
                    let mut args = vec![Val::Resource(resource)];
                    add_handle_contexts(&mut args, store.data());
                    let values =
                        trampoline::gate(&mut store, TYPES, concat!("[drop]", $name), args, real)
                            .await
                            .map_err(wasmtime::Error::new)?;
                    if !values.is_empty() {
                        return Err(wasmtime::Error::new(shape("no results")));
                    }
                    store.data_mut().remove_wasi_handle_context(id);
                    Ok(())
                })
            },
        )?;
    };
}

pub(super) use gate_http_drop;
