//! Decoding of an HTTP RPC response, shared by the WASM client and tests.

use serde::de::DeserializeOwned;

use crate::BcError;

/// Decodes an RPC response: `2xx` carries `T`, anything else a [`BcError`].
///
/// # Arguments
///
/// * `status` - HTTP status code.
/// * `body` - Response body text.
///
/// # Returns
///
/// The decoded value.
///
/// # Errors
///
/// Returns the server's [`BcError`] for a non-2xx status, or
/// [`BcError::Internal`] naming the status when a body does not decode.
#[inline]
pub fn decode<T>(status: u16, body: &str) -> Result<T, BcError>
where
    T: DeserializeOwned,
{
    if (200..300).contains(&status) {
        serde_json::from_str(body)
            .map_err(|e| BcError::Internal(format!("HTTP {status}: undecodable response: {e}")))
    } else {
        Err(serde_json::from_str::<BcError>(body)
            .unwrap_or_else(|_| BcError::Internal(format!("HTTP {status}: {body}"))))
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::decode;
    use crate::BcError;

    #[test]
    fn success_decodes_the_value() {
        let v: Vec<u32> = decode(200, "[1,2]").expect("ok");
        assert_eq!(v, vec![1, 2]);
    }

    #[rstest]
    #[case(409, r#"{"Conflict":"transaction tx-1 changed"}"#, BcError::Conflict("transaction tx-1 changed".to_owned()))]
    #[case(422, r#"{"Validation":"bad date"}"#, BcError::Validation("bad date".to_owned()))]
    #[case(404, r#"{"NotFound":"tx-1"}"#, BcError::NotFound("tx-1".to_owned()))]
    fn an_error_status_decodes_the_bc_error(
        #[case] status: u16,
        #[case] body: &str,
        #[case] expected: BcError,
    ) {
        assert_eq!(decode::<()>(status, body), Err(expected));
    }

    #[test]
    fn a_non_json_error_body_becomes_internal_with_the_status() {
        let err = decode::<()>(502, "<html>Bad Gateway</html>").expect_err("err");
        assert!(
            matches!(err, BcError::Internal(ref m) if m.contains("502")),
            "{err:?}"
        );
    }

    #[test]
    fn an_undecodable_success_body_is_internal() {
        let err = decode::<u32>(200, "\"not a number\"").expect_err("err");
        assert!(matches!(err, BcError::Internal(_)), "{err:?}");
    }
}
