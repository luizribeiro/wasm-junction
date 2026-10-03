use std::collections::HashMap;

use wasm_junction_core::{CallError, FutureHandle, InvocationId};

pub(crate) struct ActiveFutures<T> {
    next: u64,
    values: HashMap<u64, T>,
}

impl<T> ActiveFutures<T> {
    pub(crate) fn insert(
        &mut self,
        value: T,
        invocation: InvocationId,
    ) -> Result<FutureHandle, CallError> {
        let id = self.next;
        self.next = id
            .checked_add(1)
            .ok_or_else(|| CallError::trap("component future identifier space exhausted"))?;
        self.values.insert(id, value);
        Ok(FutureHandle::__for_invocation(id, invocation))
    }

    pub(crate) fn take(
        &mut self,
        handle: &FutureHandle,
        invocation: InvocationId,
    ) -> Result<T, CallError> {
        if handle.invocation_id() != invocation {
            return Err(CallError::refused(format!(
                "future {} does not belong to this invocation",
                handle.id()
            )));
        }
        self.values
            .remove(&handle.id())
            .ok_or_else(|| CallError::refused(format!("unknown future handle {}", handle.id())))
    }
}

impl<T> Default for ActiveFutures<T> {
    fn default() -> Self {
        Self {
            next: 1,
            values: HashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_junction_core::Val;
    use wasmtime::component::FutureReader;
    use wasmtime::{Config, Engine, Store};

    #[test]
    fn futures_round_trip_once_within_their_invocation() {
        let first = InvocationId::__from_counter(1);
        let second = InvocationId::__from_counter(2);
        let mut config = Config::new();
        config.concurrency_support(true);
        let engine = Engine::new(&config).unwrap();
        let mut store = Store::new(&engine, ());
        let reader =
            FutureReader::new(&mut store, async { Ok::<_, wasmtime::Error>(7_u32) }).unwrap();
        let future = reader.try_into_future_any(&mut store).unwrap();
        let mut futures = ActiveFutures::default();
        let Val::Future(handle) = Val::Future(futures.insert(future, first).unwrap()) else {
            unreachable!()
        };

        let foreign = futures.take(&handle, second).unwrap_err();
        assert!(foreign.to_string().contains("does not belong"));
        let future = futures.take(&handle, first).unwrap();
        let mut reader = future.try_into_future_reader::<u32>().unwrap();
        reader.close(&mut store).unwrap();
        let stale = futures.take(&handle, first).unwrap_err();
        assert!(stale.to_string().contains("unknown future"));
    }
}
