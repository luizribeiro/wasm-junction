use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use wasm_junction::WasiSettings;

#[derive(Clone)]
pub(crate) struct Settings {
    authority: String,
    environment: BTreeMap<String, String>,
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
        let environment = [
            ("OPENAI_SCHEME", scheme),
            ("OPENAI_AUTHORITY", authority.clone()),
            ("OPENAI_PATH_PREFIX", path_prefix),
            ("OPENAI_API_KEY", api_key),
            ("OPENAI_MODEL", model),
        ]
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect();
        Ok(Some(Self {
            authority,
            environment,
            private_data_hosts: parse_private_data_hosts(
                &std::env::var("PRIVATE_DATA_HOSTS").unwrap_or_default(),
            ),
        }))
    }

    pub(crate) fn wasi(&self, names: Option<&BTreeSet<String>>) -> Result<WasiSettings, String> {
        names
            .into_iter()
            .flatten()
            .try_fold(WasiSettings::new(), |wasi, name| {
                match self.environment.get(name) {
                    Some(value) => Ok(wasi.env(name, value)),
                    None => Err(format!(
                        "policy requests unknown environment variable {name}"
                    )),
                }
            })
    }

    pub(crate) fn model_host(&self) -> &str {
        &self.authority
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
    use super::{Settings, parse_base_url, parse_private_data_hosts};
    use std::collections::{BTreeMap, BTreeSet};

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

    #[test]
    fn builds_only_the_requested_environment() {
        let settings = Settings {
            authority: "model.example".into(),
            environment: BTreeMap::from([
                ("OPENAI_API_KEY".into(), "secret".into()),
                ("OPENAI_MODEL".into(), "model".into()),
            ]),
            private_data_hosts: Vec::new(),
        };
        let names = BTreeSet::from(["OPENAI_MODEL".into()]);
        assert_eq!(
            settings
                .wasi(Some(&names))
                .unwrap()
                .environment()
                .collect::<Vec<_>>(),
            [("OPENAI_MODEL", "model")]
        );
        let unknown = BTreeSet::from(["MISSING".into()]);
        assert_eq!(
            settings.wasi(Some(&unknown)).unwrap_err(),
            "policy requests unknown environment variable MISSING"
        );
    }
}
