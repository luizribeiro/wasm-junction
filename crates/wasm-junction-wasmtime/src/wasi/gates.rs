use wasm_junction_core::{CallError, Val, Vals};
use wasmtime::component::Linker;
use wasmtime_wasi::p2::bindings::clocks::wall_clock::Datetime;

use super::trampoline::{self, Real};
use crate::engine::StoreData;

trait ToVal {
    fn to_val(self) -> Val;
}

trait FromVal: Sized {
    fn from_val(value: Val) -> Result<Self, CallError>;
}

fn shape(expected: &str) -> CallError {
    CallError::trap(format!("expected {expected}"))
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

impl<T: ToVal> ToVal for Vec<T> {
    fn to_val(self) -> Val {
        Val::List(self.into_iter().map(ToVal::to_val).collect())
    }
}

impl<T: FromVal> FromVal for Vec<T> {
    fn from_val(value: Val) -> Result<Self, CallError> {
        match value {
            Val::List(items) => items.into_iter().map(T::from_val).collect(),
            _ => Err(shape("list")),
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

fn finish<T: FromVal>(outcome: Result<Vals, CallError>) -> wasmtime::Result<T> {
    let values = outcome.map_err(wasmtime::Error::new)?;
    let [value] = <[Val; 1]>::try_from(values).map_err(|_| shape("one result"))?;
    T::from_val(value).map_err(wasmtime::Error::new)
}

macro_rules! gate {
    ($linker:ident, $iface:literal, $name:literal, $view:ident, $method:path,
     ($($arg:ident: $ty:ty),*) -> $ok:ty) => {
        $linker.instance($iface)?.func_wrap_async(
            $name,
            |mut store, ($($arg,)*): ($($ty,)*)| Box::new(async move {
                let args = vec![$($arg.to_val()),*];
                let real: Real = |mut store, args| Box::pin(async move {
                    #[allow(unused_mut, unused_variables)]
                    let mut args = args.into_iter();
                    $(let $arg = <$ty>::from_val(
                        args.next().ok_or_else(|| shape("another argument"))?
                    )?;)*
                    let value = $method(&mut views::$view(store.data_mut()) $(, $arg)*)
                        .map_err(|error| CallError::trap(error.to_string()))?;
                    Ok(vec![value.to_val()])
                });
                let outcome = trampoline::gate(&mut store, $iface, $name, args, real).await;
                Ok((finish::<$ok>(outcome)?,))
            }),
        )?;
    };
}

pub(super) fn add_environment(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    gate!(linker, "wasi:cli/environment@0.2.12", "get-environment", cli,
        wasmtime_wasi::p2::bindings::cli::environment::Host::get_environment,
        () -> Vec<(String, String)>);
    gate!(linker, "wasi:cli/environment@0.2.12", "get-arguments", cli,
        wasmtime_wasi::p2::bindings::cli::environment::Host::get_arguments,
        () -> Vec<String>);
    gate!(linker, "wasi:cli/environment@0.2.12", "initial-cwd", cli,
        wasmtime_wasi::p2::bindings::cli::environment::Host::initial_cwd,
        () -> Option<String>);
    Ok(())
}

pub(super) fn add_wall_clock(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    gate!(linker, "wasi:clocks/wall-clock@0.2.12", "now", clocks,
        wasmtime_wasi::p2::bindings::clocks::wall_clock::Host::now,
        () -> Datetime);
    gate!(linker, "wasi:clocks/wall-clock@0.2.12", "resolution", clocks,
        wasmtime_wasi::p2::bindings::clocks::wall_clock::Host::resolution,
        () -> Datetime);
    Ok(())
}

mod views {
    use wasmtime_wasi::cli::WasiCliView;
    use wasmtime_wasi::clocks::WasiClocksView;

    use crate::engine::StoreData;

    pub(super) fn cli(store: &mut StoreData) -> wasmtime_wasi::cli::WasiCliCtxView<'_> {
        store.cli()
    }

    pub(super) fn clocks(store: &mut StoreData) -> wasmtime_wasi::clocks::WasiClocksCtxView<'_> {
        store.clocks()
    }
}
