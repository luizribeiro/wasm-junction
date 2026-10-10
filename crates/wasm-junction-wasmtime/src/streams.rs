use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll};

use wasm_junction_core::{
    ChannelDirection, EngineEvent, ImportDispatcher, InputStream, InvocationId, OutputStream,
    OutputStreamWriter, StreamHandle, Val,
};
use wasmtime::component::types::StreamType;
use wasmtime::component::{
    Destination, Source, StreamAny, StreamConsumer, StreamProducer, StreamReader, StreamResult,
    Type, Val as WasmtimeVal, VecBuffer,
};
use wasmtime::{AsContextMut, StoreContextMut};

use crate::engine::{StoreData, lift_future, lift_resource, lower_future, lower_resource};
use crate::values::{LiftValue, LowerValue, from_wasmtime, to_wasmtime};

#[cfg(feature = "wasi-p3")]
mod typed;

#[cfg(feature = "wasi-p3")]
pub(crate) use typed::{lift_static_stream, lower_static_stream};
pub(crate) type ActiveStreams = Arc<Mutex<HashMap<u64, (ActiveWriter, ChannelDirection)>>>;

pub(crate) enum ActiveWriter {
    Bytes(OutputStreamWriter),
    Values(OutputStreamWriter<Val>),
}

impl ActiveWriter {
    fn abort(&self) {
        match self {
            Self::Bytes(writer) => writer.abort(),
            Self::Values(writer) => writer.abort(),
        }
    }
}

pub(crate) fn abort_streams(store: &StoreData) {
    let streams = lock_active(&store.active_streams)
        .drain()
        .collect::<Vec<_>>();
    for (id, (writer, direction)) in streams {
        writer.abort();
        if let Some(invocation) = store.context.invocation_id() {
            store.imports.emit(EngineEvent::ChannelClose {
                invocation,
                stream: id,
                direction,
            });
        }
    }
}

pub(crate) fn lift_stream_with_direction(
    stream: StreamAny,
    mut store: impl AsContextMut<Data = StoreData>,
    direction: ChannelDirection,
) -> Result<StreamHandle, wasmtime::Error> {
    let reader = stream.try_into_stream_reader::<u8>()?;
    let (writer, output) = OutputStream::channel();
    let handle = StreamHandle::from(output);
    let id = handle.id();
    let imports = store.as_context().data().imports.clone();
    let invocation = invocation_id(store.as_context().data())?;
    imports.emit(EngineEvent::ChannelOpen {
        invocation,
        stream: id,
        direction,
    });
    let active = store.as_context().data().active_streams.clone();
    lock_active(&active).insert(id, (ActiveWriter::Bytes(writer.clone()), direction));
    reader.pipe(
        store.as_context_mut(),
        CoreConsumer {
            writer: Some(writer),
            id,
            invocation,
            imports,
            active,
            direction,
            closed: false,
        },
    )?;
    Ok(handle)
}

pub(crate) fn lift_stream(
    stream: StreamAny,
    mut store: impl AsContextMut<Data = StoreData>,
) -> Result<StreamHandle, wasmtime::Error> {
    let stream_type = stream
        .ty()
        .ok_or_else(|| wasmtime::Error::msg("stream has no dynamic item type"))?;
    let item_type = stream_type
        .ty()
        .ok_or_else(|| wasmtime::Error::msg("stream has no item type"))?;
    if item_type == Type::U8 {
        return lift_stream_with_direction(stream, store, ChannelDirection::GuestToHost);
    }

    let (writer, output) = OutputStream::<Val>::channel();
    let handle = output.__into_handle_with(Ok);
    let id = handle.id();
    let imports = store.as_context().data().imports.clone();
    let invocation = invocation_id(store.as_context().data())?;
    imports.emit(EngineEvent::ChannelOpen {
        invocation,
        stream: id,
        direction: ChannelDirection::GuestToHost,
    });
    let active = store.as_context().data().active_streams.clone();
    lock_active(&active).insert(
        id,
        (
            ActiveWriter::Values(writer.clone()),
            ChannelDirection::GuestToHost,
        ),
    );
    stream.pipe(
        store.as_context_mut(),
        ValConsumer {
            writer: Some(writer),
            item_type,
            id,
            invocation,
            imports,
            active,
            closed: false,
        },
    )?;
    Ok(handle)
}

pub(crate) fn lower_stream(
    handle: StreamHandle,
    store: impl AsContextMut<Data = StoreData>,
) -> Result<StreamAny, wasmtime::Error> {
    lower_stream_with_direction(handle, store, Some(ChannelDirection::HostToGuest))
}

pub(crate) fn lower_stream_with_direction(
    handle: StreamHandle,
    mut store: impl AsContextMut<Data = StoreData>,
    direction: Option<ChannelDirection>,
) -> Result<StreamAny, wasmtime::Error> {
    let id = handle.id();
    let input =
        InputStream::try_from(handle).map_err(|error| wasmtime::Error::msg(error.to_string()))?;
    let imports = store.as_context().data().imports.clone();
    let invocation = invocation_id(store.as_context().data())?;
    if let Some(direction) = direction {
        imports.emit(EngineEvent::ChannelOpen {
            invocation,
            stream: id,
            direction,
        });
    }
    let reader = StreamReader::new(
        store.as_context_mut(),
        CoreProducer {
            input: Some(input),
            id,
            invocation,
            imports,
            direction,
            closed: false,
        },
    )?;
    reader.try_into_stream_any(store.as_context_mut())
}

pub(crate) fn lower_typed_stream(
    handle: StreamHandle,
    stream_type: StreamType,
    mut store: impl AsContextMut<Data = StoreData>,
) -> Result<StreamAny, wasmtime::Error> {
    let item_type = stream_type
        .ty()
        .ok_or_else(|| wasmtime::Error::msg("stream has no item type"))?;
    if item_type == Type::U8 {
        return lower_stream(handle, store);
    }

    let id = handle.id();
    let input = InputStream::<Val>::__from_handle_with(handle, Ok)
        .map_err(|error| wasmtime::Error::msg(error.to_string()))?;
    let imports = store.as_context().data().imports.clone();
    let invocation = invocation_id(store.as_context().data())?;
    imports.emit(EngineEvent::ChannelOpen {
        invocation,
        stream: id,
        direction: ChannelDirection::HostToGuest,
    });
    let stream = StreamAny::new(
        store.as_context_mut(),
        stream_type,
        ValProducer {
            input: Some(input),
            item_type,
            id,
            invocation,
            imports,
            closed: false,
        },
    )?;
    Ok(stream)
}

pub(crate) fn recover_exported_stream(
    stream: StreamAny,
    mut store: impl AsContextMut<Data = StoreData>,
) -> Result<StreamHandle, wasmtime::Error> {
    let item_type = stream
        .ty()
        .and_then(|ty| ty.ty())
        .ok_or_else(|| wasmtime::Error::msg("stream has no item type"))?;
    if item_type == Type::U8 {
        return recover_reader(
            stream.try_into_stream_reader::<u8>()?,
            store.as_context_mut(),
        );
    }

    match stream.try_into::<ValProducer>(store.as_context_mut()) {
        Ok(mut producer) => producer
            .input
            .take()
            .map(InputStream::into_handle)
            .ok_or_else(|| wasmtime::Error::msg("returned stream is no longer readable")),
        Err(mut stream) => {
            stream.close(store.as_context_mut())?;
            Err(wasmtime::Error::new(
                wasm_junction_core::CallError::refused(
                    "guest-created streams cannot be returned because the Wasmtime store ends with each call",
                ),
            ))
        }
    }
}

fn recover_reader<T: 'static>(
    reader: StreamReader<T>,
    mut store: StoreContextMut<'_, StoreData>,
) -> Result<StreamHandle, wasmtime::Error> {
    match reader.try_into::<StreamHandle>(store.as_context_mut()) {
        Ok(handle) => Ok(handle),
        Err(mut reader) => {
            reader.close(store.as_context_mut())?;
            Err(wasmtime::Error::new(
                wasm_junction_core::CallError::refused(
                    "guest-created streams cannot be returned because the Wasmtime store ends with each call",
                ),
            ))
        }
    }
}

struct CoreProducer {
    input: Option<InputStream>,
    id: u64,
    invocation: InvocationId,
    imports: Arc<dyn ImportDispatcher>,
    direction: Option<ChannelDirection>,
    closed: bool,
}

impl CoreProducer {
    fn close(&mut self) {
        if let Some(input) = self.input.take() {
            input.close_reader();
        }
        close_channel(
            &mut self.closed,
            &self.imports,
            self.invocation,
            self.id,
            self.direction,
            None,
        );
    }
}

impl Drop for CoreProducer {
    fn drop(&mut self) {
        self.close();
    }
}

impl StreamProducer<StoreData> for CoreProducer {
    type Item = u8;
    type Buffer = VecBuffer<u8>;

    fn poll_produce<'a>(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        mut store: StoreContextMut<'a, StoreData>,
        mut destination: Destination<'a, Self::Item, Self::Buffer>,
        finish: bool,
    ) -> Poll<Result<StreamResult, wasmtime::Error>> {
        let this = self.get_mut();
        if finish {
            this.close();
            return Poll::Ready(Ok(StreamResult::Cancelled));
        }
        if destination.remaining(store.as_context_mut()) == Some(0) {
            return Poll::Ready(Ok(StreamResult::Completed));
        }
        let polled = {
            let Some(input) = &mut this.input else {
                return Poll::Ready(Ok(StreamResult::Dropped));
            };
            let future = input.read();
            let mut future = std::pin::pin!(future);
            future.as_mut().poll(context)
        };
        match polled {
            Poll::Ready(Ok(Some(bytes))) => {
                destination.set_buffer(bytes.into());
                Poll::Ready(Ok(StreamResult::Completed))
            }
            Poll::Ready(Ok(None)) => {
                this.close();
                Poll::Ready(Ok(StreamResult::Dropped))
            }
            Poll::Ready(Err(error)) => {
                this.close();
                Poll::Ready(Err(wasmtime::Error::msg(error.to_string())))
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn try_into(mut me: Pin<Box<Self>>, ty: TypeId) -> Result<Box<dyn Any>, Pin<Box<Self>>> {
        if ty != TypeId::of::<StreamHandle>() {
            return Err(me);
        }
        let Some(input) = me.as_mut().get_mut().input.take() else {
            return Err(me);
        };
        let handle = input.into_handle();
        me.as_mut().get_mut().close();
        Ok(Box::new(handle))
    }
}

struct ValProducer {
    input: Option<InputStream<Val>>,
    item_type: Type,
    id: u64,
    invocation: InvocationId,
    imports: Arc<dyn ImportDispatcher>,
    closed: bool,
}

impl ValProducer {
    fn close(&mut self) {
        if let Some(input) = self.input.take() {
            input.close_reader();
        }
        close_channel(
            &mut self.closed,
            &self.imports,
            self.invocation,
            self.id,
            Some(ChannelDirection::HostToGuest),
            None,
        );
    }
}

impl Drop for ValProducer {
    fn drop(&mut self) {
        self.close();
    }
}

impl StreamProducer<StoreData> for ValProducer {
    type Item = WasmtimeVal;
    type Buffer = VecBuffer<WasmtimeVal>;

    fn poll_produce<'a>(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        mut store: StoreContextMut<'a, StoreData>,
        mut destination: Destination<'a, Self::Item, Self::Buffer>,
        finish: bool,
    ) -> Poll<Result<StreamResult, wasmtime::Error>> {
        let this = self.get_mut();
        if finish {
            this.close();
            return Poll::Ready(Ok(StreamResult::Cancelled));
        }
        if destination.remaining(store.as_context_mut()) == Some(0) {
            return Poll::Ready(Ok(StreamResult::Completed));
        }
        let polled = {
            let Some(input) = this.input.as_mut() else {
                return Poll::Ready(Ok(StreamResult::Dropped));
            };
            let future = input.read();
            std::pin::pin!(future).poll(context)
        };
        match polled {
            Poll::Ready(Ok(Some(values))) => {
                let mut converted = Vec::with_capacity(values.len());
                let mut lowered = Vec::with_capacity(values.len());
                for value in values {
                    match lower_value(value.clone(), &this.item_type, &mut store) {
                        Ok(item) => {
                            converted.push(item);
                            lowered.push(value);
                        }
                        Err(error) => {
                            restore_owned_resources(&lowered, store.data_mut());
                            this.close();
                            return Poll::Ready(Err(error));
                        }
                    }
                }
                destination.set_buffer(converted.into());
                Poll::Ready(Ok(StreamResult::Completed))
            }
            Poll::Ready(Ok(None)) => {
                this.close();
                Poll::Ready(Ok(StreamResult::Dropped))
            }
            Poll::Ready(Err(error)) => {
                this.close();
                Poll::Ready(Err(wasmtime::Error::msg(error.to_string())))
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn try_into(me: Pin<Box<Self>>, ty: TypeId) -> Result<Box<dyn Any>, Pin<Box<Self>>> {
        if ty != TypeId::of::<Self>() {
            return Err(me);
        }
        let producer: Box<Self> = Pin::into_inner(me);
        Ok(producer)
    }
}

struct ValConsumer {
    writer: Option<OutputStreamWriter<Val>>,
    item_type: Type,
    id: u64,
    invocation: InvocationId,
    imports: Arc<dyn ImportDispatcher>,
    active: ActiveStreams,
    closed: bool,
}

impl ValConsumer {
    fn close(&mut self) {
        self.writer.take();
        close_channel(
            &mut self.closed,
            &self.imports,
            self.invocation,
            self.id,
            Some(ChannelDirection::GuestToHost),
            Some(&self.active),
        );
    }
}

impl Drop for ValConsumer {
    fn drop(&mut self) {
        self.close();
    }
}

impl StreamConsumer<StoreData> for ValConsumer {
    type Item = WasmtimeVal;

    fn poll_consume(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        mut store: StoreContextMut<'_, StoreData>,
        mut source: Source<'_, Self::Item>,
        finish: bool,
    ) -> Poll<Result<StreamResult, wasmtime::Error>> {
        let this = self.get_mut();
        let mut items = Vec::with_capacity(source.remaining(store.as_context_mut()));
        source.read_vals(store.as_context_mut(), &mut items)?;
        let mut values = Vec::with_capacity(items.len());
        for item in items {
            match lift_value(item, &this.item_type, &mut store) {
                Ok(value) => values.push(value),
                Err(error) => {
                    restore_owned_resources(&values, store.data_mut());
                    if let Some(writer) = &this.writer {
                        writer.abort();
                    }
                    this.close();
                    return Poll::Ready(Err(error));
                }
            }
        }
        if !values.is_empty() {
            let Some(writer) = this.writer.clone() else {
                restore_owned_resources(&values, store.data_mut());
                return Poll::Ready(Ok(StreamResult::Dropped));
            };
            let future = writer.write(values.clone());
            match std::pin::pin!(future).poll(context) {
                Poll::Ready(Ok(())) => {}
                Poll::Ready(Err(_)) => {
                    restore_owned_resources(&values, store.data_mut());
                    this.close();
                    return Poll::Ready(Ok(StreamResult::Dropped));
                }
                Poll::Pending => return Poll::Pending,
            }
        }
        if finish {
            this.close();
            Poll::Ready(Ok(StreamResult::Dropped))
        } else {
            Poll::Ready(Ok(StreamResult::Completed))
        }
    }
}

fn lift_value(
    value: WasmtimeVal,
    expected: &Type,
    store: &mut StoreContextMut<'_, StoreData>,
) -> Result<Val, wasmtime::Error> {
    from_wasmtime(value, Some(expected), &mut |value| match value {
        LiftValue::Resource(resource) => {
            lift_resource(resource, store.as_context_mut()).map(Val::Resource)
        }
        LiftValue::Future(future) => lift_future(future, store.data_mut()).map(Val::Future),
        LiftValue::Stream(stream) => lift_stream(stream, store.as_context_mut()).map(Val::Stream),
    })
}

fn lower_value(
    value: Val,
    expected: &Type,
    store: &mut StoreContextMut<'_, StoreData>,
) -> Result<WasmtimeVal, wasmtime::Error> {
    to_wasmtime(value, Some(expected), &mut |value| match value {
        LowerValue::Resource(resource, expected) => {
            lower_resource(&resource, expected, store.as_context_mut()).map(WasmtimeVal::Resource)
        }
        LowerValue::Future(future) => {
            lower_future(&future, store.data_mut()).map(WasmtimeVal::Future)
        }
        LowerValue::Stream(stream, ty) => {
            lower_typed_stream(stream, ty, store.as_context_mut()).map(WasmtimeVal::Stream)
        }
    })
}

fn restore_owned_resources(values: &[Val], store: &mut StoreData) {
    for value in values {
        match value {
            Val::Resource(resource)
                if resource.ownership() == wasm_junction_core::ResourceOwnership::Own =>
            {
                store.owned_resources.insert(resource.clone());
            }
            Val::List(items) | Val::Tuple(items) => restore_owned_resources(items, store),
            Val::Record(fields) => {
                for (_, value) in fields {
                    restore_owned_resources(std::slice::from_ref(value), store);
                }
            }
            Val::Variant {
                value: Some(value), ..
            }
            | Val::Option(Some(value))
            | Val::Result(Ok(Some(value)) | Err(Some(value))) => {
                restore_owned_resources(std::slice::from_ref(value), store);
            }
            _ => {}
        }
    }
}

struct CoreConsumer {
    writer: Option<OutputStreamWriter>,
    id: u64,
    invocation: InvocationId,
    imports: Arc<dyn ImportDispatcher>,
    active: ActiveStreams,
    direction: ChannelDirection,
    closed: bool,
}

impl CoreConsumer {
    fn close(&mut self) {
        self.writer.take();
        close_channel(
            &mut self.closed,
            &self.imports,
            self.invocation,
            self.id,
            Some(self.direction),
            Some(&self.active),
        );
    }
}

fn close_channel(
    closed: &mut bool,
    imports: &Arc<dyn ImportDispatcher>,
    invocation: InvocationId,
    id: u64,
    direction: Option<ChannelDirection>,
    active: Option<&ActiveStreams>,
) {
    let was_active = active.is_none_or(|active| lock_active(active).remove(&id).is_some());
    if std::mem::replace(closed, true) || !was_active {
        return;
    }
    if let Some(direction) = direction {
        imports.emit(EngineEvent::ChannelClose {
            invocation,
            stream: id,
            direction,
        });
    }
}

pub(super) fn invocation_id(store: &StoreData) -> Result<InvocationId, wasmtime::Error> {
    store
        .context
        .invocation_id()
        .ok_or_else(|| wasmtime::Error::msg("stream has no invocation id"))
}

pub(super) fn lock_active(
    active: &ActiveStreams,
) -> MutexGuard<'_, HashMap<u64, (ActiveWriter, ChannelDirection)>> {
    match active.lock() {
        Ok(streams) => streams,
        Err(poisoned) => poisoned.into_inner(),
    }
}

impl Drop for CoreConsumer {
    fn drop(&mut self) {
        self.close();
    }
}

impl StreamConsumer<StoreData> for CoreConsumer {
    type Item = u8;

    fn poll_consume(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        store: StoreContextMut<'_, StoreData>,
        source: Source<'_, Self::Item>,
        finish: bool,
    ) -> Poll<Result<StreamResult, wasmtime::Error>> {
        let this = self.get_mut();
        let mut source = source.as_direct(store);
        let bytes = source.remaining().to_vec();
        source.mark_read(bytes.len());
        if !bytes.is_empty() {
            let polled = {
                let Some(writer) = &this.writer else {
                    return Poll::Ready(Ok(StreamResult::Dropped));
                };
                let future = writer.write(bytes);
                let mut future = std::pin::pin!(future);
                future.as_mut().poll(context)
            };
            match polled {
                Poll::Ready(Ok(())) => {}
                Poll::Ready(Err(_)) => {
                    this.close();
                    return Poll::Ready(Ok(StreamResult::Dropped));
                }
                Poll::Pending => return Poll::Pending,
            }
        }
        if finish {
            this.close();
            Poll::Ready(Ok(StreamResult::Dropped))
        } else {
            Poll::Ready(Ok(StreamResult::Completed))
        }
    }
}
