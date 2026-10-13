use std::path::Path;

use wasm_junction::WasiSettings;

#[derive(Clone)]
pub(crate) struct Settings {
    scheme: String,
    authority: String,
    path_prefix: String,
    api_key: String,
    model: String,
    pub(crate) private_data_hosts: Vec<String>,
}

impl Settings {
    pub(crate) fn read() -> Result<Option<Self>, String> {
        let (Ok(base_url), Ok(api_key), Ok(model)) = (
            std::env::var("OPENAI_BASE_URL"),
            std::env::var("OPENAI_API_KEY"),
            std::env::var("OPENAI_MODEL"),
        ) else {
            return Ok(None);
        };
        let (scheme, authority, path_prefix) = parse_base_url(&base_url)?;
        Ok(Some(Self {
            scheme,
            authority,
            path_prefix,
            api_key,
            model,
            private_data_hosts: parse_private_data_hosts(
                &std::env::var("PRIVATE_DATA_HOSTS").unwrap_or_default(),
            ),
        }))
    }

    pub(crate) fn wasi(&self) -> WasiSettings {
        WasiSettings::new()
            .network(true)
            .env("OPENAI_SCHEME", &self.scheme)
            .env("OPENAI_AUTHORITY", &self.authority)
            .env("OPENAI_PATH_PREFIX", &self.path_prefix)
            .env("OPENAI_API_KEY", &self.api_key)
            .env("OPENAI_MODEL", &self.model)
    }
}

pub(crate) fn load_dotenv() -> Result<(), dotenvy::Error> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(".env");
    match dotenvy::from_path(path) {
        Ok(()) => Ok(()),
        Err(dotenvy::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn parse_base_url(url: &str) -> Result<(String, String, String), String> {
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| "OPENAI_BASE_URL has no scheme".to_owned())?;
    if !matches!(scheme, "http" | "https") {
        return Err("OPENAI_BASE_URL must use HTTP or HTTPS".into());
    }
    let (authority, path) = rest.find('/').map_or((rest, ""), |at| rest.split_at(at));
    if authority.is_empty() {
        return Err("OPENAI_BASE_URL has no authority".into());
    }
    Ok((
        scheme.into(),
        authority.into(),
        path.trim_end_matches('/').into(),
    ))
}

fn parse_private_data_hosts(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|host| !host.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{parse_base_url, parse_private_data_hosts};

    #[test]
    fn parses_base_urls() {
        assert_eq!(
            parse_base_url("https://example.test/v1/").unwrap(),
            ("https".into(), "example.test".into(), "/v1".into())
        );
        assert!(parse_base_url("ftp://example.test").is_err());
        assert!(parse_base_url("https:///v1").is_err());
        assert!(parse_base_url("example.test/v1").is_err());
    }

    #[test]
    fn parses_private_data_hosts() {
        assert_eq!(
            parse_private_data_hosts(" local.test:8000,api.test,, "),
            ["local.test:8000", "api.test"]
        );
    }
}
