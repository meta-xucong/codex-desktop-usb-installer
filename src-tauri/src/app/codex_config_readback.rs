//! Strict local readback of the Codex TOML written by the USB selection bridge.
use toml::Value;
use url::Url;

pub(crate) const USB_404TOKEN_PROVIDER_ID: &str = "404token-sub2api";
const USB_CODEX_MODEL_PROVIDER_ID: &str = "404token";
const READBACK_MISMATCH: &str = "selection-config-readback-mismatch";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CodexConfigReadback {
    pub active_model_provider: String,
    pub selected_model: String,
    pub base_url: String,
    pub wire_api: String,
}

fn expected_codex_model_provider(provider_id: &str) -> Option<&'static str> {
    match provider_id {
        USB_404TOKEN_PROVIDER_ID => Some(USB_CODEX_MODEL_PROVIDER_ID),
        _ => None,
    }
}

fn canonical_base_url(raw: &str) -> Option<String> {
    let mut url = Url::parse(raw.trim()).ok()?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let path = format!("{}/", url.path().trim_end_matches('/'));
    url.set_path(&path);
    Some(url.to_string())
}

fn selected_profile(document: &Value) -> Result<Option<&Value>, &'static str> {
    let Some(profile_name) = document.get("profile") else {
        return Ok(None);
    };
    let profile_name = profile_name
        .as_str()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or(READBACK_MISMATCH)?;
    let profile = document
        .get("profiles")
        .and_then(Value::as_table)
        .and_then(|profiles| profiles.get(profile_name))
        .filter(|profile| profile.is_table())
        .ok_or(READBACK_MISMATCH)?;
    Ok(Some(profile))
}

fn effective_string<'a>(
    document: &'a Value,
    profile: Option<&'a Value>,
    key: &str,
) -> Option<&'a str> {
    profile
        .and_then(|profile| profile.get(key))
        .or_else(|| document.get(key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub(crate) fn validate(
    bytes: &[u8],
    binding_provider_id: &str,
    expected_model: &str,
    expected_base_url: &str,
) -> Result<CodexConfigReadback, &'static str> {
    let text = std::str::from_utf8(bytes).map_err(|_| READBACK_MISMATCH)?;
    let document: Value = text.parse().map_err(|_| READBACK_MISMATCH)?;
    let profile = selected_profile(&document)?;
    let active = effective_string(&document, profile, "model_provider").ok_or(READBACK_MISMATCH)?;
    if expected_codex_model_provider(binding_provider_id) != Some(active) {
        return Err(READBACK_MISMATCH);
    }
    let provider = document
        .get("model_providers")
        .and_then(Value::as_table)
        .and_then(|providers| providers.get(active))
        .and_then(Value::as_table)
        .ok_or(READBACK_MISMATCH)?;
    let selected_model = effective_string(&document, profile, "model").ok_or(READBACK_MISMATCH)?;
    if selected_model != expected_model {
        return Err(READBACK_MISMATCH);
    }
    let actual_base_url = provider
        .get("base_url")
        .and_then(Value::as_str)
        .ok_or(READBACK_MISMATCH)?;
    let expected_base_url = canonical_base_url(expected_base_url).ok_or(READBACK_MISMATCH)?;
    let actual_base_url = canonical_base_url(actual_base_url).ok_or(READBACK_MISMATCH)?;
    if actual_base_url != expected_base_url {
        return Err(READBACK_MISMATCH);
    }
    if provider.get("wire_api").and_then(Value::as_str) != Some("responses")
        || provider
            .get("requires_openai_auth")
            .and_then(Value::as_bool)
            != Some(true)
    {
        return Err(READBACK_MISMATCH);
    }
    Ok(CodexConfigReadback {
        active_model_provider: active.to_string(),
        selected_model: selected_model.to_string(),
        base_url: actual_base_url,
        wire_api: "responses".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE_URL: &str = "https://404token.xyz/v1";
    const MODEL: &str = "fixture/vendor-model";

    fn valid_config() -> String {
        format!(
            "model_provider = \"404token\"\nmodel = \"{MODEL}\"\n\n[model_providers.404token]\nbase_url = \"{BASE_URL}\"\nwire_api = \"responses\"\nrequires_openai_auth = true\n"
        )
    }

    #[test]
    fn accepts_the_bound_provider_model_endpoint_and_protocol() {
        let config = valid_config();
        let readback = validate(config.as_bytes(), USB_404TOKEN_PROVIDER_ID, MODEL, BASE_URL)
            .expect("valid USB config should pass");
        assert_eq!(readback.active_model_provider, "404token");
        assert_eq!(readback.selected_model, MODEL);
        assert_eq!(readback.base_url, "https://404token.xyz/v1/");
        assert_eq!(readback.wire_api, "responses");
    }

    #[test]
    fn accepts_an_equivalent_trailing_slash_but_not_a_query_or_userinfo() {
        let config = valid_config().replace(BASE_URL, "https://404token.xyz/v1/");
        assert!(validate(config.as_bytes(), USB_404TOKEN_PROVIDER_ID, MODEL, BASE_URL,).is_ok());
        for endpoint in [
            "https://404token.xyz/v1?wrong=1",
            "https://attacker:secret@404token.xyz/v1",
            "http://404token.xyz/v1",
        ] {
            let config = valid_config().replace(BASE_URL, endpoint);
            assert!(
                validate(config.as_bytes(), USB_404TOKEN_PROVIDER_ID, MODEL, BASE_URL,).is_err()
            );
        }
    }

    #[test]
    fn rejects_wrong_provider_and_model() {
        let config = valid_config();
        assert!(validate(config.as_bytes(), "other-provider", MODEL, BASE_URL).is_err());
        assert!(validate(
            config.as_bytes(),
            USB_404TOKEN_PROVIDER_ID,
            "another-model",
            BASE_URL,
        )
        .is_err());
        let wrong_provider = config.replace(
            "model_provider = \"404token\"",
            "model_provider = \"wrong-provider\"",
        );
        assert!(validate(
            wrong_provider.as_bytes(),
            USB_404TOKEN_PROVIDER_ID,
            MODEL,
            BASE_URL,
        )
        .is_err());
    }

    #[test]
    fn rejects_wrong_wire_api_auth_requirement_and_malformed_toml() {
        for replacement in [
            ("wire_api = \"responses\"", "wire_api = \"chat\""),
            (
                "requires_openai_auth = true",
                "requires_openai_auth = false",
            ),
        ] {
            let config = valid_config().replace(replacement.0, replacement.1);
            assert!(
                validate(config.as_bytes(), USB_404TOKEN_PROVIDER_ID, MODEL, BASE_URL,).is_err()
            );
        }
        assert!(validate(
            b"model_provider = \"404token\"\n[model_providers.404token]\nbase_url = [",
            USB_404TOKEN_PROVIDER_ID,
            MODEL,
            BASE_URL,
        )
        .is_err());
    }

    #[test]
    fn rejects_missing_active_provider_model_or_endpoint() {
        for config in [
            "model = \"fixture/vendor-model\"\n",
            "model_provider = \"404token\"\nmodel = \"fixture/vendor-model\"\n",
            "model_provider = \"404token\"\n[model_providers.other]\nbase_url = \"https://404token.xyz/v1\"\n",
        ] {
            assert!(validate(
                config.as_bytes(),
                USB_404TOKEN_PROVIDER_ID,
                MODEL,
                BASE_URL,
            )
            .is_err());
        }
    }

    #[test]
    fn active_profile_is_used_for_provider_and_model_readback() {
        let matching = format!(
            "profile = \"work\"\n{}\n[profiles.work]\nmodel_provider = \"404token\"\nmodel = \"{MODEL}\"\n",
            valid_config()
        );
        assert!(validate(
            matching.as_bytes(),
            USB_404TOKEN_PROVIDER_ID,
            MODEL,
            BASE_URL
        )
        .is_ok());

        let wrong_provider = format!(
            "profile = \"work\"\n{}\n[profiles.work]\nmodel_provider = \"other-provider\"\nmodel = \"{MODEL}\"\n",
            valid_config()
        );
        assert!(validate(
            wrong_provider.as_bytes(),
            USB_404TOKEN_PROVIDER_ID,
            MODEL,
            BASE_URL,
        )
        .is_err());

        let wrong_model = format!(
            "profile = \"work\"\n{}\n[profiles.work]\nmodel_provider = \"404token\"\nmodel = \"another-model\"\n",
            valid_config()
        );
        assert!(validate(
            wrong_model.as_bytes(),
            USB_404TOKEN_PROVIDER_ID,
            MODEL,
            BASE_URL,
        )
        .is_err());
    }

    #[test]
    fn rejects_a_selected_profile_that_is_missing_or_malformed() {
        for config in [
            "profile = \"missing\"\nmodel_provider = \"404token\"\nmodel = \"fixture/vendor-model\"\n",
            "profile = 7\nmodel_provider = \"404token\"\nmodel = \"fixture/vendor-model\"\n",
            "profile = \" \"\nmodel_provider = \"404token\"\nmodel = \"fixture/vendor-model\"\n",
        ] {
            assert!(validate(
                config.as_bytes(),
                USB_404TOKEN_PROVIDER_ID,
                MODEL,
                BASE_URL,
            )
            .is_err());
        }
    }
}
