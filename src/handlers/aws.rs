use quick_xml::se::to_string;
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Serialize)]
#[serde(rename = "ErrorResponse")]
struct ErrorResponse {
    #[serde(rename = "Error")]
    error: ErrorDetails,
    #[serde(rename = "RequestId")]
    request_id: String,
}

#[derive(Serialize)]
struct ErrorDetails {
    #[serde(rename = "Code")]
    code: String,
    #[serde(rename = "Message")]
    message: String,
}

pub fn json_error(code: &str, message: &str) -> Value {
    json!({
        "__type": code,
        "Message": message
    })
}

pub fn xml_error(code: &str, message: &str) -> String {
    let error_xml = ErrorResponse {
        error: ErrorDetails {
            code: code.to_string(),
            message: message.to_string(),
        },
        request_id: "mock-id".to_string(),
    };

    to_string(&error_xml).unwrap_or_else(|_| {
        format!(
            "<ErrorResponse><Error><Code>{}</Code><Message>{}</Message></Error><RequestId>mock-id</RequestId></ErrorResponse>",
            code, message
        )
    })
}
