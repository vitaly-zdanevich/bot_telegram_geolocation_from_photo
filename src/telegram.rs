use anyhow::{Context, anyhow};
use serde::{Deserialize, Serialize};

/// Telegram webhook update with the message fields this bot needs.
#[derive(Clone, Debug, Deserialize)]
pub struct Update {
    /// Incoming message update.
    pub message: Option<Message>,
}

/// Telegram message subset handled by the bot.
#[derive(Clone, Debug, Deserialize)]
pub struct Message {
    /// Message identifier used for replies.
    pub message_id: i64,
    /// Chat where the message was sent.
    pub chat: Chat,
    /// Text command or plain text, when present.
    pub text: Option<String>,
    /// Normal Telegram photos. These are not original files.
    #[serde(default)]
    pub photo: Vec<PhotoSize>,
    /// Original file/document upload.
    pub document: Option<Document>,
    /// User-shared Telegram location.
    pub location: Option<Location>,
}

/// Telegram chat identifier.
#[derive(Clone, Debug, Deserialize)]
pub struct Chat {
    /// Numeric chat ID.
    pub id: i64,
}

/// A Telegram photo variant.
#[derive(Clone, Debug, Deserialize)]
pub struct PhotoSize {
    /// File ID for this resized Telegram photo.
    pub file_id: String,
}

/// Telegram location payload.
#[derive(Clone, Copy, Debug, Deserialize)]
pub struct Location {
    /// Latitude in decimal degrees.
    pub latitude: f64,
    /// Longitude in decimal degrees.
    pub longitude: f64,
}

/// Telegram document metadata.
#[derive(Clone, Debug, Deserialize)]
pub struct Document {
    /// File ID used to download the document.
    pub file_id: String,
    /// Original filename supplied by Telegram.
    pub file_name: Option<String>,
    /// MIME type supplied by Telegram.
    pub mime_type: Option<String>,
    /// File size in bytes.
    pub file_size: Option<u64>,
}

impl Document {
    /// Returns true when the document looks like an image file worth scanning.
    pub fn looks_like_image(&self) -> bool {
        if self
            .mime_type
            .as_deref()
            .is_some_and(|mime_type| mime_type.starts_with("image/"))
        {
            return true;
        }

        self.file_name
            .as_deref()
            .and_then(|name| name.rsplit('.').next())
            .map(|extension| {
                matches!(
                    extension.to_ascii_lowercase().as_str(),
                    "jpg"
                        | "jpeg"
                        | "tif"
                        | "tiff"
                        | "dng"
                        | "heic"
                        | "heif"
                        | "png"
                        | "avif"
                        | "cr3"
                        | "raf"
                        | "iiq"
                        | "webp"
                )
            })
            .unwrap_or(false)
    }
}

/// Telegram Bot API client.
#[derive(Clone)]
pub struct TelegramClient {
    client: reqwest::Client,
    token: String,
}

impl TelegramClient {
    /// Creates a new Telegram API client.
    pub fn new(client: reqwest::Client, token: String) -> Self {
        Self { client, token }
    }

    /// Prepares a Telegram file for download.
    pub async fn get_file(&self, file_id: &str) -> anyhow::Result<TelegramFile> {
        let url = self.method_url("getFile");
        let response = self
            .client
            .get(url)
            .query(&[("file_id", file_id)])
            .send()
            .await
            .context("Telegram getFile request failed")?
            .error_for_status()
            .context("Telegram getFile returned an error status")?
            .json::<TelegramResponse<TelegramFile>>()
            .await
            .context("failed to parse Telegram getFile response")?;

        response.into_result("getFile")
    }

    /// Downloads a file returned by `getFile`.
    pub async fn download_file(&self, file_path: &str) -> anyhow::Result<Vec<u8>> {
        let url = format!(
            "https://api.telegram.org/file/bot{}/{file_path}",
            self.token
        );
        let bytes = self
            .client
            .get(url)
            .send()
            .await
            .context("Telegram file download request failed")?
            .error_for_status()
            .context("Telegram file download returned an error status")?
            .bytes()
            .await
            .context("failed to read Telegram file bytes")?;

        Ok(bytes.to_vec())
    }

    /// Shows Telegram's typing indicator in the target chat.
    pub async fn send_typing_action(&self, chat_id: i64) -> anyhow::Result<()> {
        let request = SendChatActionRequest {
            chat_id,
            action: "typing",
        };

        let response = self
            .client
            .post(self.method_url("sendChatAction"))
            .json(&request)
            .send()
            .await
            .context("Telegram sendChatAction request failed")?
            .error_for_status()
            .context("Telegram sendChatAction returned an error status")?
            .json::<TelegramResponse<bool>>()
            .await
            .context("failed to parse Telegram sendChatAction response")?;

        response.into_result("sendChatAction").map(|_| ())
    }

    /// Sends a plain text reply.
    pub async fn send_message(
        &self,
        chat_id: i64,
        reply_to_message_id: i64,
        text: &str,
    ) -> anyhow::Result<()> {
        let request = SendMessageRequest {
            chat_id,
            text,
            reply_to_message_id,
            disable_web_page_preview: true,
        };

        let response = self
            .client
            .post(self.method_url("sendMessage"))
            .json(&request)
            .send()
            .await
            .context("Telegram sendMessage request failed")?
            .error_for_status()
            .context("Telegram sendMessage returned an error status")?
            .json::<TelegramResponse<serde_json::Value>>()
            .await
            .context("failed to parse Telegram sendMessage response")?;

        response.into_result("sendMessage").map(|_| ())
    }

    /// Sends a native Telegram location pin.
    pub async fn send_location(
        &self,
        chat_id: i64,
        reply_to_message_id: i64,
        latitude: f64,
        longitude: f64,
    ) -> anyhow::Result<()> {
        let request = SendLocationRequest {
            chat_id,
            latitude,
            longitude,
            reply_to_message_id,
        };

        let response = self
            .client
            .post(self.method_url("sendLocation"))
            .json(&request)
            .send()
            .await
            .context("Telegram sendLocation request failed")?
            .error_for_status()
            .context("Telegram sendLocation returned an error status")?
            .json::<TelegramResponse<serde_json::Value>>()
            .await
            .context("failed to parse Telegram sendLocation response")?;

        response.into_result("sendLocation").map(|_| ())
    }

    fn method_url(&self, method: &str) -> String {
        format!("https://api.telegram.org/bot{}/{method}", self.token)
    }
}

/// File metadata returned by Telegram `getFile`.
#[derive(Clone, Debug, Deserialize)]
pub struct TelegramFile {
    /// Remote path used with the Telegram file download endpoint.
    pub file_path: String,
    /// File size in bytes, if supplied.
    pub file_size: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct TelegramResponse<T> {
    ok: bool,
    result: Option<T>,
    description: Option<String>,
}

impl<T> TelegramResponse<T> {
    fn into_result(self, method: &str) -> anyhow::Result<T> {
        if self.ok {
            self.result
                .ok_or_else(|| anyhow!("Telegram {method} response did not include result"))
        } else {
            Err(anyhow!(
                "Telegram {method} failed: {}",
                self.description
                    .unwrap_or_else(|| "unknown error".to_string())
            ))
        }
    }
}

#[derive(Debug, Serialize)]
struct SendMessageRequest<'a> {
    chat_id: i64,
    text: &'a str,
    reply_to_message_id: i64,
    disable_web_page_preview: bool,
}

#[derive(Debug, Serialize)]
struct SendChatActionRequest<'a> {
    chat_id: i64,
    action: &'a str,
}

#[derive(Debug, Serialize)]
struct SendLocationRequest {
    chat_id: i64,
    latitude: f64,
    longitude: f64,
    reply_to_message_id: i64,
}

#[cfg(test)]
mod tests {
    use super::Document;

    #[test]
    fn detects_image_documents() {
        assert!(
            Document {
                file_id: "1".to_string(),
                file_name: None,
                mime_type: Some("image/jpeg".to_string()),
                file_size: None,
            }
            .looks_like_image()
        );

        assert!(
            Document {
                file_id: "1".to_string(),
                file_name: Some("IMG_1234.HEIC".to_string()),
                mime_type: None,
                file_size: None,
            }
            .looks_like_image()
        );

        assert!(
            !Document {
                file_id: "1".to_string(),
                file_name: Some("notes.txt".to_string()),
                mime_type: Some("text/plain".to_string()),
                file_size: None,
            }
            .looks_like_image()
        );
    }
}
