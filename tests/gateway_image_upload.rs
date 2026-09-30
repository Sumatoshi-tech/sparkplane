#![cfg(feature = "appliance")]

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use image::{DynamicImage, ImageFormat, ImageReader, RgbImage};
use sparkplane::spark::gateway::{self, GatewayProfile, ImageProtocol, IngressLimits};

fn profile() -> GatewayProfile {
    let config: toml::Value = toml::from_str(include_str!(
        "../configs/sparkplane/engines/vllm-qwen38-mmap.toml"
    ))
    .unwrap();
    let mut profile = GatewayProfile::text();
    profile.capabilities.insert("vision".into());
    profile.vision = Some(config["profiles"][0]["vision"].clone().try_into().unwrap());
    let vision = profile.vision.as_mut().unwrap();
    vision.max_width = 64;
    vision.max_height = 32;
    vision.max_bytes = 4096;
    vision.max_total_bytes = 4096;
    vision.max_count = 1;
    profile
}

fn png() -> Vec<u8> {
    let mut seed = 0x12345678_u32;
    let pixels = (0..1024 * 512 * 3)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as u8
        })
        .collect();
    let image = DynamicImage::ImageRgb8(RgbImage::from_raw(1024, 512, pixels).unwrap());
    let mut out = std::io::Cursor::new(Vec::new());
    image.write_to(&mut out, ImageFormat::Png).unwrap();
    out.into_inner()
}

fn responses(image: &[u8]) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"describe"},{"type":"input_image","image_url":format!("data:image/png;base64,{}",BASE64.encode(image)),"detail":"high"}]}],"stream":true,"store":false})).unwrap()
}

fn check_image(url: &str, profile: &GatewayProfile) {
    let (media, data) = url
        .strip_prefix("data:")
        .unwrap()
        .split_once(";base64,")
        .unwrap();
    let bytes = BASE64.decode(data).unwrap();
    let vision = profile.vision.as_ref().unwrap();
    assert!(vision.media_types.contains(media));
    assert!(bytes.len() <= vision.max_bytes);
    let decoded = ImageReader::new(std::io::Cursor::new(&bytes))
        .with_guessed_format()
        .unwrap()
        .decode()
        .unwrap();
    assert!(decoded.width() <= vision.max_width);
    assert!(decoded.height() <= vision.max_height);
    assert_eq!(decoded.width(), decoded.height() * 2);
}

#[test]
fn large_uploads_fit_the_model_in_both_public_protocols() {
    let image = png();
    let request = responses(&image);
    assert!(request.len() > 1024 * 1024);
    let profile = profile();
    let limits = IngressLimits::default();
    assert!(gateway::rewrite_responses_request_with_profile(&request, "served", &profile).is_err());
    let normalized =
        gateway::normalize_image_request(&request, ImageProtocol::Responses, &profile, &limits)
            .unwrap();
    let request =
        gateway::rewrite_responses_request_with_profile(&normalized, "served", &profile).unwrap();
    let request: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
    check_image(
        request["messages"][0]["content"][1]["image_url"]["url"]
            .as_str()
            .unwrap(),
        &profile,
    );
    let request=serde_json::to_vec(&serde_json::json!({"model":"public","max_tokens":8,"messages":[{"role":"user","content":[{"type":"text","text":"describe"},{"type":"image","source":{"type":"base64","media_type":"image/png","data":BASE64.encode(image)}}]}]})).unwrap();
    let normalized =
        gateway::normalize_image_request(&request, ImageProtocol::Anthropic, &profile, &limits)
            .unwrap();
    let request =
        gateway::rewrite_anthropic_request_with_profile(&normalized, "served", &profile).unwrap();
    let request: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
    check_image(
        request["messages"][0]["content"][1]["image_url"]["url"]
            .as_str()
            .unwrap(),
        &profile,
    );
}

#[test]
fn native_responses_receives_normalized_images_and_preserves_other_fields() {
    let mut profile = profile();
    profile.native_responses = true;
    let normalized = gateway::normalize_image_request(
        &responses(&png()),
        ImageProtocol::Responses,
        &profile,
        &IngressLimits::default(),
    )
    .unwrap();
    let request =
        gateway::rewrite_native_responses_request_with_profile(&normalized, "public", &profile)
            .unwrap();
    let request: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(request["store"], false);
    assert_eq!(request["input"][0]["content"][0]["text"], "describe");
    assert_eq!(request["input"][0]["content"][1]["detail"], "high");
    check_image(
        request["input"][0]["content"][1]["image_url"]
            .as_str()
            .unwrap(),
        &profile,
    );
}

#[test]
fn upload_byte_pixel_and_model_image_count_limits_stay_finite() {
    let request = responses(&png());
    let profile = profile();
    for limits in [
        IngressLimits {
            request_body_bytes: 1024,
            ..IngressLimits::default()
        },
        IngressLimits {
            image_bytes: 1024,
            ..IngressLimits::default()
        },
        IngressLimits {
            image_pixels: 1024,
            ..IngressLimits::default()
        },
        IngressLimits {
            image_dimension: 512,
            ..IngressLimits::default()
        },
    ] {
        let error =
            gateway::normalize_image_request(&request, ImageProtocol::Responses, &profile, &limits)
                .unwrap_err();
        assert_eq!(error.code, "request_too_large");
    }
    let mut request: serde_json::Value = serde_json::from_slice(&request).unwrap();
    let image = request["input"][0]["content"][1].clone();
    request["input"][0]["content"]
        .as_array_mut()
        .unwrap()
        .push(image);
    assert!(
        gateway::normalize_image_request(
            &serde_json::to_vec(&request).unwrap(),
            ImageProtocol::Responses,
            &profile,
            &IngressLimits::default()
        )
        .is_err()
    );
}

#[test]
fn compatible_images_are_unchanged_and_resizing_preserves_transparency() {
    let profile = profile();
    let image = DynamicImage::ImageRgb8(RgbImage::from_pixel(2, 1, image::Rgb([10, 20, 30])));
    let mut original = std::io::Cursor::new(Vec::new());
    image.write_to(&mut original, ImageFormat::Png).unwrap();
    let original = original.into_inner();
    let normalized = gateway::normalize_image_request(
        &responses(&original),
        ImageProtocol::Responses,
        &profile,
        &IngressLimits::default(),
    )
    .unwrap();
    let normalized: serde_json::Value = serde_json::from_slice(&normalized).unwrap();
    assert_eq!(
        normalized["input"][0]["content"][1]["image_url"],
        format!("data:image/png;base64,{}", BASE64.encode(&original))
    );

    let image = image::DynamicImage::ImageRgba8(image::RgbaImage::from_fn(128, 64, |_, _| {
        image::Rgba([200, 30, 80, 0])
    }));
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, ImageFormat::Png).unwrap();
    let normalized = gateway::normalize_image_request(
        &responses(&png.into_inner()),
        ImageProtocol::Responses,
        &profile,
        &IngressLimits::default(),
    )
    .unwrap();
    let normalized: serde_json::Value = serde_json::from_slice(&normalized).unwrap();
    let url = normalized["input"][0]["content"][1]["image_url"]
        .as_str()
        .unwrap();
    check_image(url, &profile);
    let bytes = BASE64
        .decode(url.split_once(";base64,").unwrap().1)
        .unwrap();
    let decoded = ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .unwrap()
        .decode()
        .unwrap();
    assert!(decoded.color().has_alpha());
    assert!(decoded.to_rgba8().pixels().all(|pixel| pixel[3] == 0));
}

#[test]
fn normalization_keeps_inline_only_and_verified_vision_requirements() {
    let mut profile = profile();
    let mut request: serde_json::Value = serde_json::from_slice(&responses(&png())).unwrap();
    for invalid in [
        "https://example.com/picture.png",
        "file:///tmp/picture.png",
        "data:image/jpeg;base64,iVBORw0KGgo=",
    ] {
        request["input"][0]["content"][1]["image_url"] = invalid.into();
        assert!(
            gateway::normalize_image_request(
                &serde_json::to_vec(&request).unwrap(),
                ImageProtocol::Responses,
                &profile,
                &IngressLimits::default()
            )
            .is_err()
        );
    }
    profile.vision = None;
    assert!(
        gateway::normalize_image_request(
            &responses(&png()),
            ImageProtocol::Responses,
            &profile,
            &IngressLimits::default()
        )
        .is_err()
    );
}

#[test]
fn codex_view_image_tool_results_are_images_and_share_the_model_budget() {
    let profile = profile();
    for (call, output) in [
        ("function_call", "function_call_output"),
        ("custom_tool_call", "custom_tool_call_output"),
    ] {
        let image = serde_json::json!({"type":"input_image","image_url":format!("data:image/png;base64,{}",BASE64.encode(png())),"detail":"high"});
        let request = serde_json::json!({"input":[
            {"type":"message","role":"user","content":"inspect the screenshot"},
            {"type":call,"call_id":"call_view","name":"view_image","arguments":"{}","input":"image"},
            {"type":output,"call_id":"call_view","output":[{"type":"input_text","text":"Screenshot from the tool."},image]}
        ],"stream":true});
        let normalized = gateway::normalize_image_request(
            &serde_json::to_vec(&request).unwrap(),
            ImageProtocol::Responses,
            &profile,
            &IngressLimits::default(),
        )
        .unwrap();
        let rewritten =
            gateway::rewrite_responses_request_with_profile(&normalized, "served", &profile)
                .unwrap();
        let rewritten: serde_json::Value = serde_json::from_slice(&rewritten.body).unwrap();
        let tool = &rewritten["messages"][2];
        assert_eq!(tool["role"], "tool");
        assert_eq!(tool["tool_call_id"], "call_view");
        assert!(
            tool["content"].is_array(),
            "view_image output must not become base64 text"
        );
        assert_eq!(
            tool["content"][0],
            serde_json::json!({"type":"text","text":"Screenshot from the tool."})
        );
        check_image(
            tool["content"][1]["image_url"]["url"].as_str().unwrap(),
            &profile,
        );
        let mut native = profile.clone();
        native.native_responses = true;
        let rewritten =
            gateway::rewrite_native_responses_request_with_profile(&normalized, "public", &native)
                .unwrap();
        let rewritten: serde_json::Value = serde_json::from_slice(&rewritten.body).unwrap();
        check_image(
            rewritten["input"][2]["output"][1]["image_url"]
                .as_str()
                .unwrap(),
            &native,
        );
        let mut two_images = request.clone();
        two_images["input"][2]["output"]
            .as_array_mut()
            .unwrap()
            .push(image);
        assert!(
            gateway::normalize_image_request(
                &serde_json::to_vec(&two_images).unwrap(),
                ImageProtocol::Responses,
                &profile,
                &IngressLimits::default()
            )
            .is_err()
        );
        assert!(
            gateway::normalize_image_request(
                &serde_json::to_vec(&request).unwrap(),
                ImageProtocol::Responses,
                &GatewayProfile::text(),
                &IngressLimits::default()
            )
            .is_err()
        );
    }
}

#[test]
fn successive_image_tool_results_reserve_bytes_for_every_frame() {
    let config: toml::Value = toml::from_str(include_str!(
        "../configs/sparkplane/engines/vllm-qwen38-mmap.toml"
    ))
    .unwrap();
    let mut profile = profile();
    profile.vision.as_mut().unwrap().max_count = config["profiles"][0]["vision"]["max_count"]
        .as_integer()
        .unwrap() as usize;
    assert_eq!(profile.vision.as_ref().unwrap().max_count, 16);
    let source = DynamicImage::ImageRgb8(RgbImage::from_fn(128, 64, |x, y| {
        image::Rgb([
            (x * 31 + y * 17) as u8,
            (x * 71 + y * 43) as u8,
            (x * 19 + y * 83) as u8,
        ])
    }));
    let mut encoded = std::io::Cursor::new(Vec::new());
    source.write_to(&mut encoded, ImageFormat::Png).unwrap();
    let image = serde_json::json!({"type":"input_image","image_url":format!("data:image/png;base64,{}",BASE64.encode(encoded.into_inner())),"detail":"high"});
    for count in [2, 3, 16] {
        let mut input = vec![
            serde_json::json!({"type":"message","role":"user","content":"compare the screenshots"}),
        ];
        for index in 0..count {
            let id = format!("view_{index}");
            input.push(serde_json::json!({"type":"function_call","name":"view_image","call_id":id,"arguments":"{}"}));
            input.push(serde_json::json!({"type":"function_call_output","call_id":id,"output":[{"type":"input_text","text":format!("Frame {index}")},image]}));
            if index + 1 < count {
                input.push(serde_json::json!({"type":"message","role":"assistant","content":"Now inspect the next frame."}));
            }
        }
        let request =
            serde_json::to_vec(&serde_json::json!({"input":input,"stream":true})).unwrap();
        let normalized = gateway::normalize_image_request(
            &request,
            ImageProtocol::Responses,
            &profile,
            &IngressLimits::default(),
        )
        .unwrap();
        let normalized: serde_json::Value = serde_json::from_slice(&normalized).unwrap();
        let mut total_bytes = 0;
        let mut frames = 0;
        let retained = if count == 16 { 10 } else { count };
        for (index, item) in normalized["input"].as_array().unwrap().iter().enumerate() {
            if item["type"] != "function_call_output" {
                continue;
            }
            assert_eq!(
                item["output"][0]["text"],
                format!("Frame {}", (index - 1) / 3)
            );
            let Some(url) = item["output"][1]["image_url"].as_str() else {
                continue;
            };
            let bytes = BASE64
                .decode(url.split_once(";base64,").unwrap().1)
                .unwrap();
            assert!(bytes.len() <= profile.vision.as_ref().unwrap().max_total_bytes / retained);
            total_bytes += bytes.len();
            frames += 1;
        }
        assert_eq!(frames, retained);
        assert!(total_bytes <= profile.vision.as_ref().unwrap().max_total_bytes);
        let bytes = serde_json::to_vec(&normalized).unwrap();
        assert!(
            gateway::rewrite_responses_request_with_profile(&bytes, "served", &profile).is_ok()
        );
        let mut native = profile.clone();
        native.native_responses = true;
        assert!(
            gateway::rewrite_native_responses_request_with_profile(&bytes, "served", &native)
                .is_ok()
        );
    }
}

#[test]
fn multiple_images_share_the_gateway_pixel_limit_and_keep_a_finite_count() {
    let mut profile = profile();
    profile.vision.as_mut().unwrap().max_count = 16;
    let source = DynamicImage::ImageRgb8(RgbImage::from_pixel(128, 64, image::Rgb([10, 20, 30])));
    let mut encoded = std::io::Cursor::new(Vec::new());
    source.write_to(&mut encoded, ImageFormat::Png).unwrap();
    let image = serde_json::json!({"type":"input_image","image_url":format!("data:image/png;base64,{}",BASE64.encode(encoded.into_inner()))});
    let make_request = |count| {
        serde_json::to_vec(&serde_json::json!({"input":[{"role":"user","type":"message","content":vec![image.clone();count]}]})).unwrap()
    };
    let limits = IngressLimits {
        image_pixels: 128 * 64 + 1,
        ..IngressLimits::default()
    };
    assert!(
        gateway::normalize_image_request(
            &make_request(1),
            ImageProtocol::Responses,
            &profile,
            &limits
        )
        .is_ok()
    );
    assert_eq!(
        gateway::normalize_image_request(
            &make_request(2),
            ImageProtocol::Responses,
            &profile,
            &limits
        )
        .unwrap_err()
        .code,
        "request_too_large"
    );
    assert_eq!(
        gateway::normalize_image_request(
            &make_request(17),
            ImageProtocol::Responses,
            &profile,
            &IngressLimits::default()
        )
        .unwrap_err()
        .message,
        "image count exceeds the model profile limit"
    );
}

#[test]
fn anthropic_nested_image_results_keep_calls_and_recent_frames() {
    let mut profile = profile();
    profile.vision.as_mut().unwrap().max_count = 16;
    let image = BASE64.encode(png());
    let mut messages = Vec::new();
    for index in 0..20 {
        messages.push(serde_json::json!({"role":"assistant","content":[{"type":"text","text":"Inspect another frame."},{"type":"tool_use","id":format!("t{index}"),"name":"view_image","input":{"path":format!("/tmp/{index}.png")}}]}));
        messages.push(serde_json::json!({"role":"user","content":[{"type":"tool_result","tool_use_id":format!("t{index}"),"content":[{"type":"text","text":format!("Frame {index}")},{"type":"image","source":{"type":"base64","media_type":"image/png","data":image}}]}]}));
    }
    let bytes = serde_json::to_vec(
        &serde_json::json!({"model":"public","max_tokens":8,"messages":messages}),
    )
    .unwrap();
    // Use small originals so the source body fits normal direct gateway ingress.
    let source = gateway::normalize_image_request(
        &bytes,
        ImageProtocol::Anthropic,
        &profile,
        &IngressLimits::default(),
    )
    .unwrap_err();
    assert_eq!(source.code, "request_too_large");
    let image = &profile.vision.as_ref().unwrap().health_image_base64;
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    for message in value["messages"].as_array_mut().unwrap() {
        if message["role"] == "user" {
            message["content"][0]["content"][1]["source"]["data"] = image.clone().into();
        }
    }
    let normalized = gateway::normalize_image_request(
        &serde_json::to_vec(&value).unwrap(),
        ImageProtocol::Anthropic,
        &profile,
        &IngressLimits::default(),
    )
    .unwrap();
    let rewritten =
        gateway::rewrite_anthropic_request_with_profile(&normalized, "served", &profile).unwrap();
    let rewritten: serde_json::Value = serde_json::from_slice(&rewritten.body).unwrap();
    let tools = rewritten["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| message["role"] == "tool")
        .collect::<Vec<_>>();
    assert_eq!(tools.len(), 20);
    assert_eq!(tools.last().unwrap()["tool_call_id"], "t19");
    assert!(
        tools[0]["content"]
            .as_str()
            .unwrap()
            .contains("Older image")
    );
    assert_eq!(tools[19]["content"][0]["text"], "Frame 19");
    assert_eq!(
        tools
            .iter()
            .filter(|tool| tool["content"].is_array())
            .count(),
        8
    );
}

#[test]
fn chat_vision_uses_the_same_retention_and_finite_profile_budget() {
    let mut profile = profile();
    profile.vision.as_mut().unwrap().max_count = 16;
    let url = format!(
        "data:image/png;base64,{}",
        profile.vision.as_ref().unwrap().health_image_base64
    );
    let mut messages = Vec::new();
    for index in 0..30 {
        messages.push(serde_json::json!({"role":"user","content":[{"type":"text","text":format!("Frame {index}")},{"type":"image_url","image_url":{"url":url}}]}));
        if index < 29 {
            messages.push(serde_json::json!({"role":"assistant","content":"Observed frame."}));
        }
    }
    let normalized = gateway::normalize_image_request(
        &serde_json::to_vec(&serde_json::json!({"messages":messages})).unwrap(),
        ImageProtocol::Chat,
        &profile,
        &IngressLimits::default(),
    )
    .unwrap();
    assert!(gateway::rewrite_chat_request_with_profile(&normalized, "served", &profile).is_ok());
    assert!(
        gateway::rewrite_chat_request_with_profile(&normalized, "served", &GatewayProfile::text())
            .is_err()
    );
}
