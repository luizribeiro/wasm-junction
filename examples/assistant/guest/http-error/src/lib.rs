use wasip3::http::types::ErrorCode;

pub fn message(error: &ErrorCode) -> String {
    match error {
        ErrorCode::HttpRequestDenied => "request denied by policy".into(),
        ErrorCode::InternalError(Some(message)) => message.clone(),
        _ => plain(error),
    }
}

pub fn plain(value: &impl std::fmt::Debug) -> String {
    let debug = format!("{value:?}");
    let name = debug
        .strip_prefix("ErrorCode::")
        .unwrap_or(&debug)
        .split('(')
        .next()
        .unwrap_or(&debug);
    let characters = name.chars().collect::<Vec<_>>();
    let mut words = String::new();
    for (index, character) in characters.iter().copied().enumerate() {
        let previous = index.checked_sub(1).and_then(|at| characters.get(at));
        let next = characters.get(index + 1);
        if index > 0
            && character.is_uppercase()
            && (previous.is_some_and(|character| character.is_lowercase())
                || next.is_some_and(|character| character.is_lowercase())
                    && previous.is_some_and(|character| character.is_uppercase()))
        {
            words.push(' ');
        }
        words.extend(character.to_lowercase());
    }
    words
}
