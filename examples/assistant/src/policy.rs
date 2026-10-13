#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Event {
    caller: String,
    callee: String,
    interface: String,
    function: String,
    arguments: Vec<String>,
}

mod event;

#[cfg(test)]
mod tests;
