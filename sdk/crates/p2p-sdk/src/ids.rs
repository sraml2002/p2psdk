//! IDS HTTP REST client (registerIds, queryIds, sendIceOffer).

use p2p_io::traits::HttpTransport;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Deserialize)]
#[allow(non_snake_case)]
pub struct IdsRecord {
    pub appId: String,
    pub userId: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub odid: String,
    #[serde(default)]
    pub token: String,
}

// ---------------------------------------------------------------------------
// API functions
// ---------------------------------------------------------------------------

/// Register a device with the IDS service.
pub fn register_ids(
    http: &dyn HttpTransport,
    host: &str,
    app_id: &str,
    user_id: &str,
    type_: &str,
    odid: &str,
    token: &str,
) -> Result<(), String> {
    let url = format!("{host}/api/ids");
    let body = serde_json::json!({
        "appId": app_id,
        "userId": user_id,
        "type": type_,
        "odid": odid,
        "token": token,
    });
    let body_bytes = serde_json::to_vec(&body).map_err(|e| e.to_string())?;
    let headers = headers();
    let (status, resp) = http.post(&url, &headers, &body_bytes).map_err(|e| format!("HTTP: {e}"))?;
    if !(200..300).contains(&status) {
        return Err(format!("IDS register failed: HTTP {status}"));
    }
    let env: serde_json::Value = serde_json::from_str(&resp).map_err(|e| e.to_string())?;
    let code = env["code"].as_u64().unwrap_or(0);
    if code != 200 {
        let msg = env["message"].as_str().unwrap_or("unknown");
        return Err(format!("IDS register error: code={code}, msg={msg}"));
    }
    Ok(())
}

/// Query the IDS service for a user's records.
///
/// Returns the first service-type record found.
pub fn query_ids(
    http: &dyn HttpTransport,
    host: &str,
    app_id: &str,
    user_id: &str,
) -> Result<IdsRecord, String> {
    let url = format!("{host}/api/ids/{app_id}/{user_id}");
    let headers = headers();
    let (status, resp) = http.get(&url, &headers).map_err(|e| format!("HTTP: {e}"))?;
    if !(200..300).contains(&status) {
        return Err(format!("IDS query failed: HTTP {status}"));
    }

    #[derive(serde::Deserialize)]
    struct Envelope {
        code: u64,
        #[allow(dead_code)]
        message: String,
        data: Vec<IdsRecord>,
    }

    let env: Envelope = serde_json::from_str(&resp).map_err(|e| e.to_string())?;
    if env.code != 200 {
        return Err(format!("IDS query error: code={}", env.code));
    }
    env.data
        .into_iter()
        .find(|r| r.type_ == "service")
        .ok_or_else(|| "No service record found in IDS".into())
}

/// Send an SDP offer to the peer's ICE service.
///
/// Posts raw SDP text to `http://{peer_addr}/api/ice/offer` and returns the SDP answer.
pub fn send_ice_offer(
    http: &dyn HttpTransport,
    peer_addr: &str,
    sdp_offer: &str,
) -> Result<String, String> {
    let url = format!("http://{peer_addr}/api/ice/offer");
    let headers = vec![("Content-Type".into(), "application/sdp".into())];
    let (status, resp) = http.post(&url, &headers, sdp_offer.as_bytes())
        .map_err(|e| format!("HTTP: {e}"))?;
    if !(200..300).contains(&status) {
        return Err(format!("ICE offer failed: HTTP {status}"));
    }
    let answer = resp.trim().to_string();
    if answer.is_empty() {
        return Err("ICE offer: empty SDP answer".into());
    }
    Ok(answer)
}

fn headers() -> Vec<(String, String)> {
    vec![("Content-Type".into(), "application/json".into())]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ids_record_deserialize() {
        let json = r#"{"appId":"a","userId":"u","type":"service","odid":"o","token":"t"}"#;
        let rec: IdsRecord = serde_json::from_str(json).unwrap();
        assert_eq!(rec.appId, "a");
        assert_eq!(rec.type_, "service");
        assert_eq!(rec.token, "t");
    }
}
