//! Bounded model-visible image history, shared by the launch adapter and gateway.
//! Original tool calls, observations and instructions are never summarized here.
use std::{cell::Cell, collections::BTreeMap, io::Read, path::PathBuf, rc::Rc};

use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const REQUEST_BYTES: usize = 32 * 1024 * 1024;
const TEXT_BYTES: usize = 8 * 1024 * 1024;
const RECENT: usize = 6;
const HIGH_WATER: usize = 12;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Responses,
    Anthropic,
    Chat,
}
impl Protocol {
    fn field(self) -> &'static str {
        if self == Self::Responses {
            "input"
        } else {
            "messages"
        }
    }
    fn image(self, part: &Value) -> bool {
        part["type"]
            == match self {
                Self::Responses => "input_image",
                Self::Anthropic => "image",
                Self::Chat => "image_url",
            }
    }
    fn text(self, text: String) -> Value {
        json!({"type": if self == Self::Responses { "input_text" } else { "text" }, "text":text})
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Statistics {
    pub evicted: usize,
    pub removed_bytes: usize,
}
pub type Archive<'a> = dyn FnMut(&str, &Value, Protocol) -> Option<PathBuf> + 'a;

pub fn visit_parts(value: &mut Value, protocol: Protocol, visit: &mut impl FnMut(&mut Value)) {
    match value {
        Value::Array(parts) => {
            for part in parts {
                visit_parts(part, protocol, visit);
            }
        }
        Value::Object(_) if protocol.image(value) => visit(value),
        Value::Object(object) => {
            // Restrict traversal to protocol content; tool arguments and metadata are data.
            for field in ["content", "output"] {
                if let Some(parts) = object.get_mut(field) {
                    visit_parts(parts, protocol, visit);
                }
            }
        }
        _ => {}
    }
}
fn acknowledged(item: &Value, protocol: Protocol) -> bool {
    item["role"] == "assistant"
        || (protocol == Protocol::Responses
            && matches!(
                item["type"].as_str(),
                Some("function_call" | "custom_tool_call")
            ))
}
fn image_payload(part: &Value, protocol: Protocol) -> Option<&str> {
    match protocol {
        Protocol::Responses => part["image_url"].as_str(),
        Protocol::Anthropic => part["source"]["data"].as_str(),
        Protocol::Chat => part["image_url"]["url"].as_str(),
    }
}
pub fn image_id(part: &Value, protocol: Protocol) -> Option<String> {
    image_payload(part, protocol).map(|payload| format!("{:x}", Sha256::digest(payload.as_bytes())))
}

fn evictable(part: &Value, protocol: Protocol) -> bool {
    let Some(object) = part.as_object() else {
        return false;
    };
    match protocol {
        Protocol::Responses | Protocol::Chat => {
            if object
                .keys()
                .any(|key| !matches!(key.as_str(), "type" | "image_url" | "detail"))
            {
                return false;
            }
            let Some(payload) = image_payload(part, protocol) else {
                return false;
            };
            payload.starts_with("data:image/png;base64,")
                || payload.starts_with("data:image/jpeg;base64,")
                || payload.starts_with("data:image/webp;base64,")
        }
        Protocol::Anthropic => {
            object
                .keys()
                .all(|key| matches!(key.as_str(), "type" | "source"))
                && part["source"]["type"] == "base64"
                && matches!(
                    part["source"]["media_type"].as_str(),
                    Some("image/png" | "image/jpeg" | "image/webp")
                )
        }
    }
}

fn prune_items(
    items: &mut [Value],
    protocol: Protocol,
    max_count: usize,
    archive: &mut Option<&mut Archive<'_>>,
    enforce: bool,
) -> Result<Statistics, &'static str> {
    let ack = items.iter().rposition(|item| acknowledged(item, protocol));
    let mut total = 0;
    let mut unseen = 0;
    let mut payload_bytes = 0;
    for (index, item) in items.iter_mut().enumerate() {
        visit_parts(item, protocol, &mut |part| {
            total += 1;
            payload_bytes += image_payload(part, protocol).map(str::len).unwrap_or(0);
            if ack.is_none_or(|ack| index >= ack) {
                unseen += 1;
            }
        });
    }
    if enforce && unseen > max_count {
        return Err("new image batch exceeds the model limit; split the batch");
    }
    let high = HIGH_WATER.min(max_count);
    if total < high && payload_bytes <= REQUEST_BYTES / 2 {
        return Ok(Statistics::default());
    }
    let keep = RECENT.min(max_count).max(unseen);
    let mut remove = total.saturating_sub(keep);
    let mut stats = Statistics::default();
    let mut paths = BTreeMap::new();
    for (index, item) in items.iter_mut().enumerate() {
        if matches!(
            item["type"].as_str(),
            Some("function_call" | "custom_tool_call")
        ) && let (Some(id), Some(arguments)) =
            (item["call_id"].as_str(), item["arguments"].as_str())
            && let Ok(arguments) = serde_json::from_str::<Value>(arguments)
            && let Some(path) = arguments["path"].as_str().filter(|path| path.len() <= 4096)
        {
            paths.insert(id.to_owned(), path.to_owned());
        }
        let path = item["call_id"]
            .as_str()
            .and_then(|id| paths.get(id))
            .cloned();
        if ack.is_none_or(|ack| index >= ack) {
            continue;
        }
        if item["role"] != "user"
            && !(protocol == Protocol::Responses
                && matches!(
                    item["type"].as_str(),
                    Some("function_call_output" | "custom_tool_call_output")
                ))
            && !(protocol == Protocol::Chat && item["role"] == "tool")
        {
            continue;
        }
        visit_parts(item, protocol, &mut |part| {
            if (remove == 0 && payload_bytes <= REQUEST_BYTES / 2) || !evictable(part, protocol) {
                return;
            }
            let Some(id) = image_id(part, protocol) else {
                return;
            };
            let saved = archive
                .as_deref_mut()
                .and_then(|archive| archive(&id, part, protocol));
            let original = saved
                .map(|path| path.to_string_lossy().into_owned())
                .or_else(|| path.clone());
            let reference = match original {
                Some(path) => format!("Reopen with view_image using path {}. A source path may have changed; prefer the archived copy when present.", json!(path)),
                None => "Original is not available through this gateway; request the image again if details are needed.".into(),
            };
            let bytes = image_payload(part, protocol).map(str::len).unwrap_or(0);
            stats.removed_bytes += bytes;
            payload_bytes = payload_bytes.saturating_sub(bytes);
            stats.evicted += 1;
            remove = remove.saturating_sub(1);
            *part = protocol.text(format!("[Older image {id} omitted from active history. {reference} Written observations are retained.]"));
        });
    }
    Ok(stats)
}

/// Project an already buffered gateway request. Never evict images awaiting inspection.
pub fn project(
    value: &mut Value,
    protocol: Protocol,
    max_count: usize,
    mut archive: Option<&mut Archive<'_>>,
) -> Result<Statistics, &'static str> {
    let Some(items) = value
        .get_mut(protocol.field())
        .and_then(Value::as_array_mut)
    else {
        return Ok(Statistics::default());
    };
    let ack = items.iter().rposition(|item| acknowledged(item, protocol));
    let mut unseen = 0;
    for (index, item) in items.iter_mut().enumerate() {
        if ack.is_none_or(|ack| index >= ack) {
            visit_parts(item, protocol, &mut |_| unseen += 1);
        }
    }
    if unseen > max_count {
        return Err("new image batch exceeds the model limit; split the batch");
    }
    let original = std::mem::take(items);
    let mut stats = Statistics::default();
    for item in original {
        items.push(item);
        let next = prune_items(items, protocol, max_count, &mut archive, false)?;
        stats.evicted += next.evicted;
        stats.removed_bytes += next.removed_bytes;
    }
    let next = prune_items(items, protocol, max_count, &mut archive, true)?;
    stats.evicted += next.evicted;
    stats.removed_bytes += next.removed_bytes;
    Ok(stats)
}

struct CountWriter(usize);
impl std::io::Write for CountWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len());
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn size(value: &Value) -> usize {
    let mut writer = CountWriter(0);
    let _ = serde_json::to_writer(&mut writer, value);
    writer.0
}
struct ItemReader<R> {
    reader: R,
    count: Rc<Cell<usize>>,
    deadline: std::time::Instant,
}
impl<R: Read> Read for ItemReader<R> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if std::time::Instant::now() >= self.deadline {
            return Err(std::io::Error::other("history processing timed out"));
        }
        let count = self.reader.read(out)?;
        let total = self.count.get().saturating_add(count);
        self.count.set(total);
        if total > REQUEST_BYTES {
            return Err(std::io::Error::other(
                "individual history item exceeds 32 MiB",
            ));
        }
        Ok(count)
    }
}
struct RequestSeed<'a, 'b> {
    protocol: Protocol,
    max_count: usize,
    archive: Option<&'a mut Archive<'b>>,
    count: Rc<Cell<usize>>,
    stats: Statistics,
}
impl<'de> DeserializeSeed<'de> for &mut RequestSeed<'_, '_> {
    type Value = Value;
    fn deserialize<D: de::Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
        deserializer.deserialize_map(self)
    }
}
impl<'de> Visitor<'de> for &mut RequestSeed<'_, '_> {
    type Value = Value;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("an inference request object")
    }
    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Value, M::Error> {
        let mut object = serde_json::Map::new();
        let mut other_bytes = 0;
        while let Some(key) = map.next_key::<String>()? {
            if object.contains_key(&key) {
                return Err(de::Error::custom("duplicate request field"));
            }
            self.count.set(0);
            let value = if key == self.protocol.field() {
                map.next_value_seed(HistorySeed(&mut *self))?
            } else {
                map.next_value()?
            };
            if key != self.protocol.field() {
                other_bytes += key.len() + size(&value);
                if other_bytes > TEXT_BYTES {
                    return Err(de::Error::custom("request text exceeds 8 MiB"));
                }
            }
            object.insert(key, value);
        }
        Ok(Value::Object(object))
    }
    fn visit_seq<S: SeqAccess<'de>>(self, mut sequence: S) -> Result<Value, S::Error> {
        let mut items = Vec::new();
        let mut text_bytes = 0;
        loop {
            self.count.set(0);
            let Some(mut item) = sequence.next_element::<Value>()? else {
                break;
            };
            let mut image_bytes = 0;
            visit_parts(&mut item, self.protocol, &mut |part| {
                image_bytes += image_payload(part, self.protocol)
                    .map(str::len)
                    .unwrap_or(0);
            });
            text_bytes += size(&item).saturating_sub(image_bytes);
            if text_bytes > TEXT_BYTES {
                return Err(de::Error::custom(
                    "history text exceeds 8 MiB; compact the conversation",
                ));
            }
            items.push(item);
            let stats = prune_items(
                &mut items,
                self.protocol,
                self.max_count,
                &mut self.archive,
                false,
            )
            .map_err(de::Error::custom)?;
            self.stats.evicted += stats.evicted;
            self.stats.removed_bytes += stats.removed_bytes;
            // Bound retained memory independently of the number of discarded wire bytes.
            let mut retained = 0;
            for item in &mut items {
                visit_parts(item, self.protocol, &mut |part| {
                    retained += image_payload(part, self.protocol)
                        .map(str::len)
                        .unwrap_or(0);
                });
            }
            if retained > REQUEST_BYTES {
                return Err(de::Error::custom("active image batch exceeds 32 MiB"));
            }
        }
        let stats = prune_items(
            &mut items,
            self.protocol,
            self.max_count,
            &mut self.archive,
            true,
        )
        .map_err(de::Error::custom)?;
        self.stats.evicted += stats.evicted;
        self.stats.removed_bytes += stats.removed_bytes;
        Ok(Value::Array(items))
    }
    fn visit_str<E: de::Error>(self, text: &str) -> Result<Value, E> {
        Ok(text.into())
    }
}
struct HistorySeed<'a, 'b, 'c>(&'a mut RequestSeed<'b, 'c>);
impl<'de> DeserializeSeed<'de> for HistorySeed<'_, '_, '_> {
    type Value = Value;
    fn deserialize<D: de::Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
        deserializer.deserialize_any(&mut *self.0)
    }
}

/// Stream old inline payloads through a bounded item buffer; the full source body
/// is never retained. The adapter also imposes a finite upload/processing deadline.
pub fn project_reader<R: Read>(
    reader: R,
    protocol: Protocol,
    max_count: usize,
    archive: Option<&mut Archive<'_>>,
) -> Result<(Value, Statistics), &'static str> {
    let count = Rc::new(Cell::new(0));
    let reader = ItemReader {
        reader,
        count: count.clone(),
        deadline: std::time::Instant::now() + std::time::Duration::from_secs(120),
    };
    let mut deserializer = serde_json::Deserializer::from_reader(reader);
    let mut seed = RequestSeed {
        protocol,
        max_count,
        archive,
        count,
        stats: Statistics::default(),
    };
    let mut value = (&mut seed)
        .deserialize(&mut deserializer)
        .map_err(|error| {
            let diagnostic = error.to_string();
            if diagnostic.starts_with("new image batch exceeds") {
                "new image batch exceeds the model limit; split the batch"
            } else if diagnostic.starts_with("history text exceeds")
                || diagnostic.starts_with("request text exceeds")
            {
                "history text exceeds the adapter budget; compact the conversation"
            } else if error.is_io() || diagnostic.starts_with("active image batch exceeds") {
                "image or request exceeds the adapter byte budget"
            } else {
                "request JSON is invalid"
            }
        })?;
    deserializer
        .end()
        .map_err(|_| "request has trailing data")?;
    let stats = project(&mut value, protocol, max_count, None)?;
    seed.stats.evicted += stats.evicted;
    seed.stats.removed_bytes += stats.removed_bytes;
    if size(&value) > REQUEST_BYTES {
        return Err("projected request exceeds 32 MiB; compact the conversation");
    }
    Ok((value, seed.stats))
}
