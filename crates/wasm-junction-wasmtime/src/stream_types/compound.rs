use super::StreamTypeVisitor;
use crate::stream_values::StreamValue;

pub(super) struct WrapList<V>(pub(super) V);
pub(super) struct WrapOption<V>(pub(super) V);

impl<V: StreamTypeVisitor> StreamTypeVisitor for WrapList<V> {
    type Output = V::Output;
    fn visit<T: StreamValue>(self) -> Result<Self::Output, wasmtime::Error> {
        self.0.visit::<Vec<T>>()
    }
}

impl<V: StreamTypeVisitor> StreamTypeVisitor for WrapOption<V> {
    type Output = V::Output;
    fn visit<T: StreamValue>(self) -> Result<Self::Output, wasmtime::Error> {
        self.0.visit::<Option<T>>()
    }
}
