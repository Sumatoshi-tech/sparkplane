//! Public image uploads have separate limits from qualified engine inputs.
use std::io::{self, Cursor, Write};

use base64::Engine as _;
use image::{
    DynamicImage, ImageDecoder, ImageEncoder, ImageFormat, ImageReader, imageops::FilterType,
};
use serde_json::Value;

use super::{
    BASE64, GatewayProfile, ImageBudget, MAX_COMPLETION_BODY_BYTES, OpenAiError, VisionPolicy,
    decode_image, invalid_request, parse_data_uri,
};

pub const HARD_REQUEST_BYTES: usize = 64 * 1024 * 1024;
const DECODE_MEMORY_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct IngressLimits {
    pub request_body_bytes: usize,
    pub image_bytes: usize,
    pub image_pixels: u64,
    pub image_dimension: u32,
    pub max_parallel_requests: usize,
}

impl Default for IngressLimits {
    fn default() -> Self {
        Self {
            request_body_bytes: 32 * 1024 * 1024,
            image_bytes: 16 * 1024 * 1024,
            image_pixels: 32_000_000,
            image_dimension: 16384,
            max_parallel_requests: 2,
        }
    }
}

impl IngressLimits {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.request_body_bytes == 0
            || self.request_body_bytes > HARD_REQUEST_BYTES
            || self.image_bytes == 0
            || self.image_bytes > 32 * 1024 * 1024
            || self.image_pixels == 0
            || self.image_pixels > 32_000_000
            || self.image_dimension == 0
            || self.image_dimension > 16384
            || !(1..=4).contains(&self.max_parallel_requests)
        {
            return Err("gateway ingress limits exceed supported resource bounds");
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub enum ImageProtocol {
    Responses,
    Anthropic,
    Chat,
}

fn too_large(message: &'static str) -> OpenAiError {
    OpenAiError {
        code: "request_too_large",
        message,
    }
}

pub fn normalize_image_request(
    bytes: &[u8],
    protocol: ImageProtocol,
    profile: &GatewayProfile,
    limits: &IngressLimits,
) -> Result<Vec<u8>, OpenAiError> {
    limits.validate().map_err(invalid_request)?;
    if bytes.len() > limits.request_body_bytes {
        return Err(too_large("request exceeds the gateway upload byte limit"));
    }
    let mut request: Value =
        serde_json::from_slice(bytes).map_err(|_| invalid_request("request JSON is invalid"))?;
    let history_protocol = match protocol {
        ImageProtocol::Responses => super::super::image_history::Protocol::Responses,
        ImageProtocol::Anthropic => super::super::image_history::Protocol::Anthropic,
        ImageProtocol::Chat => super::super::image_history::Protocol::Chat,
    };
    if let Some(vision) = &profile.vision {
        super::super::image_history::project(
            &mut request,
            history_protocol,
            vision.max_count,
            None,
        )
        .map_err(|_| invalid_request("image count exceeds the model profile limit"))?;
    }
    let field = match protocol {
        ImageProtocol::Responses => "input",
        ImageProtocol::Anthropic | ImageProtocol::Chat => "messages",
    };
    let mut image_count = 0;
    super::super::image_history::visit_parts(&mut request[field], history_protocol, &mut |_| {
        image_count += 1
    });
    let mut image_policy = profile.vision.clone();
    if image_count > 0 {
        let policy = image_policy
            .as_mut()
            .ok_or_else(|| invalid_request("image input requires a vision profile"))?;
        if image_count > policy.max_count {
            return Err(invalid_request(
                "image count exceeds the model profile limit",
            ));
        }
        // Reserve an equal share for each frame, including later tool results.
        // The strict rewriter still checks the original shared engine budget.
        policy.max_bytes = policy.max_bytes.min(policy.max_total_bytes / image_count);
    }
    let mut budget = ImageBudget::default();
    if let Some(messages) = request.get_mut(field).and_then(Value::as_array_mut) {
        for message in messages {
            let user = message["role"] == "user";
            let tool = (matches!(protocol, ImageProtocol::Responses)
                && matches!(
                    message["type"].as_str(),
                    Some("function_call_output" | "custom_tool_call_output")
                ))
                || (matches!(protocol, ImageProtocol::Chat) && message["role"] == "tool");
            let mut error = None;
            super::super::image_history::visit_parts(message, history_protocol, &mut |part| {
                if error.is_some() {
                    return;
                }
                if !user && !tool {
                    error = Some(invalid_request(
                        "image input must be inline user or tool-result content",
                    ));
                    return;
                }
                let prepared = (|| {
                    let (media, data) = match protocol {
                        ImageProtocol::Responses | ImageProtocol::Chat => {
                            let uri = if matches!(protocol, ImageProtocol::Responses) {
                                part["image_url"].as_str()
                            } else {
                                part["image_url"]["url"].as_str()
                            }
                            .ok_or_else(|| {
                                invalid_request("image_url must be an inline data URI")
                            })?;
                            parse_data_uri(uri)?
                        }
                        ImageProtocol::Anthropic => {
                            let source = &part["source"];
                            if source["type"] != "base64" {
                                return Err(invalid_request(
                                    "image source must be inline base64 user content",
                                ));
                            }
                            (
                                source["media_type"].as_str().ok_or_else(|| {
                                    invalid_request("image media type is required")
                                })?,
                                source["data"]
                                    .as_str()
                                    .ok_or_else(|| invalid_request("image base64 is required"))?,
                            )
                        }
                    };
                    prepare_image(media, data, image_policy.as_ref(), &mut budget, limits)
                })();
                match prepared {
                    Ok(image) => match protocol {
                        ImageProtocol::Responses => {
                            part["image_url"] = format!(
                                "data:{};base64,{}",
                                image.media_type,
                                BASE64.encode(&image.bytes)
                            )
                            .into()
                        }
                        ImageProtocol::Chat => {
                            part["image_url"]["url"] = format!(
                                "data:{};base64,{}",
                                image.media_type,
                                BASE64.encode(&image.bytes)
                            )
                            .into()
                        }
                        ImageProtocol::Anthropic => {
                            part["source"]["media_type"] = image.media_type.into();
                            part["source"]["data"] = BASE64.encode(&image.bytes).into();
                        }
                    },
                    Err(problem) => error = Some(problem),
                }
            });
            if let Some(error) = error {
                return Err(error);
            }
        }
    }
    let bytes =
        serde_json::to_vec(&request).map_err(|_| invalid_request("request cannot be encoded"))?;
    if bytes.len() > MAX_COMPLETION_BODY_BYTES {
        return Err(too_large(
            "normalized request exceeds the 8 MiB inference body limit",
        ));
    }
    Ok(bytes)
}

fn prepare_image(
    media: &str,
    encoded: &str,
    policy: Option<&VisionPolicy>,
    budget: &mut ImageBudget,
    limits: &IngressLimits,
) -> Result<super::super::upstream::ImagePart, OpenAiError> {
    let policy = policy.ok_or_else(|| invalid_request("image input requires a vision profile"))?;
    if budget.count >= policy.max_count {
        return Err(invalid_request(
            "image count exceeds the model profile limit",
        ));
    }
    if !policy.media_types.contains(media) {
        return Err(invalid_request(
            "image format is unsupported by the model profile",
        ));
    }
    if encoded.len() > limits.image_bytes.div_ceil(3) * 4 {
        return Err(too_large("image exceeds the gateway upload byte limit"));
    }
    let bytes = BASE64
        .decode(encoded)
        .map_err(|_| invalid_request("image base64 is invalid"))?;
    if bytes.is_empty() {
        return Err(invalid_request("image data is empty"));
    }
    if bytes.len() > limits.image_bytes {
        return Err(too_large("image exceeds the gateway upload byte limit"));
    }
    let format = match media {
        "image/png" if bytes.starts_with(b"\x89PNG\r\n\x1a\n") => ImageFormat::Png,
        "image/jpeg" if bytes.starts_with(b"\xff\xd8\xff") => ImageFormat::Jpeg,
        "image/webp"
            if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP".as_slice()) =>
        {
            ImageFormat::WebP
        }
        _ => {
            return Err(invalid_request(
                "image media type does not match its magic bytes",
            ));
        }
    };
    let mut decode_limits = image::Limits::default();
    decode_limits.max_image_width = Some(limits.image_dimension);
    decode_limits.max_image_height = Some(limits.image_dimension);
    decode_limits.max_alloc = Some(DECODE_MEMORY_BYTES);
    let mut reader = ImageReader::with_format(Cursor::new(&bytes), format);
    reader.limits(decode_limits);
    let mut decoder = reader.into_decoder().map_err(|error| match error {
        image::ImageError::Limits(_) => {
            too_large("image exceeds the gateway dimension or decode memory limit")
        }
        _ => invalid_request("image header is invalid"),
    })?;
    let (width, height) = decoder.dimensions();
    let pixels = u64::from(width) * u64::from(height);
    if width == 0
        || height == 0
        || pixels > limits.image_pixels.saturating_sub(budget.source_pixels)
        || decoder.total_bytes() > DECODE_MEMORY_BYTES
    {
        return Err(too_large(
            "images exceed the gateway total pixel or decode memory limit",
        ));
    }
    budget.source_pixels += pixels;
    let available = policy
        .max_total_bytes
        .saturating_sub(budget.bytes)
        .min(policy.max_bytes);
    if available == 0 || policy.max_width == 0 || policy.max_height == 0 {
        return Err(invalid_request("image cannot fit the model profile limits"));
    }
    if bytes.len() <= available && width <= policy.max_width && height <= policy.max_height {
        return decode_image(media, encoded, Some(policy), budget);
    }
    let orientation = decoder
        .orientation()
        .map_err(|_| invalid_request("image orientation metadata is invalid"))?;
    let mut decoded = DynamicImage::from_decoder(decoder)
        .map_err(|_| invalid_request("image pixels are invalid"))?;
    decoded.apply_orientation(orientation);
    let alpha = decoded.color().has_alpha();
    let mut image = decoded.resize(
        policy.max_width.min(decoded.width()),
        policy.max_height.min(decoded.height()),
        FilterType::Lanczos3,
    );
    drop(decoded);
    image = if alpha {
        DynamicImage::ImageRgba8(image.to_rgba8())
    } else {
        DynamicImage::ImageRgb8(image.to_rgb8())
    };
    for _ in 0..16 {
        let mut formats = Vec::new();
        if alpha || format == ImageFormat::Png {
            formats.push(("image/png", ImageFormat::Png, 0));
        }
        if !alpha {
            formats.extend([
                ("image/jpeg", ImageFormat::Jpeg, 90),
                ("image/jpeg", ImageFormat::Jpeg, 80),
            ]);
        }
        formats.push(("image/webp", ImageFormat::WebP, 0));
        for (media, format, quality) in formats {
            if !policy.media_types.contains(media) {
                continue;
            }
            if let Some(bytes) = encode(&image, format, quality, available)? {
                return decode_image(media, &BASE64.encode(&bytes), Some(policy), budget);
            }
        }
        if image.width() == 1 && image.height() == 1 {
            break;
        }
        let width = (image.width() * 3 / 4).max(1);
        let height = (image.height() * 3 / 4).max(1);
        image = image.resize(width, height, FilterType::Lanczos3);
    }
    Err(invalid_request("image cannot fit the model profile limits"))
}

struct CappedWriter {
    bytes: Vec<u8>,
    limit: usize,
    exceeded: bool,
}
impl Write for CappedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            self.exceeded = true;
            return Err(io::Error::other("image encoding byte limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn encode(
    image: &DynamicImage,
    format: ImageFormat,
    quality: u8,
    limit: usize,
) -> Result<Option<Vec<u8>>, OpenAiError> {
    let mut output = CappedWriter {
        bytes: Vec::new(),
        limit,
        exceeded: false,
    };
    let result = match format {
        ImageFormat::Png => image::codecs::png::PngEncoder::new(&mut output).write_image(
            image.as_bytes(),
            image.width(),
            image.height(),
            image.color().into(),
        ),
        ImageFormat::Jpeg => {
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut output, quality)
                .encode_image(image)
        }
        ImageFormat::WebP => image::codecs::webp::WebPEncoder::new_lossless(&mut output).encode(
            image.as_bytes(),
            image.width(),
            image.height(),
            image.color().into(),
        ),
        _ => return Err(invalid_request("image output format is unsupported")),
    };
    if output.exceeded {
        return Ok(None);
    }
    result.map_err(|_| invalid_request("image cannot be encoded"))?;
    Ok(Some(output.bytes))
}
