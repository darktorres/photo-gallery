//! The answers read: an upload's ids (JSON), the results page's other sizes (its HTML-escaped state), and the
//! similar pages (JSON). Pure functions of the bodies; no request is made here.

use serde_json::Value;

use crate::types::{OtherSize, Similar, SimilarItem, SimilarPage, Upload, YandexError};

/// The ids in an upload's answer (JSON), which the result requests need.
pub fn parse_upload(body: &str) -> Result<Upload, YandexError> {
    let v: Value = serde_json::from_str(body).map_err(|_| YandexError::Unexpected("upload is not JSON"))?;
    let cbir_id = v.get("cbir_id").and_then(Value::as_str).ok_or(YandexError::Unexpected("upload has no cbir_id"))?;
    let orig_url = v
        .pointer("/sizes/orig/path")
        .and_then(Value::as_str)
        .ok_or(YandexError::Unexpected("upload has no original URL"))?;
    Ok(Upload { cbir_id: cbir_id.to_string(), orig_url: orig_url.to_string() })
}

/// Whether a response body is Yandex's anti-robot page (a captcha), whatever its status.
pub fn is_captcha_page(body: &str) -> bool {
    body.contains("showcaptcha")
}

/// Reads the pages `fetch` gives, from page 0 up to `max_pages`, as [`Client::similar`](crate::Client::similar) does. A failure on page 0 is
/// the error; one on a later page stops the list with the items read so far.
pub fn read_similar_pages(
    max_pages: u32,
    mut fetch: impl FnMut(u32) -> Result<SimilarPage, YandexError>,
) -> Result<Similar, YandexError> {
    let mut items = Vec::new();
    for page in 0..max_pages {
        let parsed = match fetch(page) {
            Ok(p) => p,
            Err(e) if page > 0 => return Ok(Similar { items, stopped: Some(e) }),
            Err(e) => return Err(e),
        };
        if parsed.items.is_empty() {
            break;
        }
        items.extend(parsed.items);
        if !parsed.has_next {
            break;
        }
    }
    Ok(Similar { items, stopped: None })
}

/// The results page's `ImagesApp-*` element carries its state as an HTML-escaped `data-state` attribute.
#[expect(clippy::string_slice, reason = "every index is a find result or the end of an ASCII pattern: a char boundary")]
fn extract_data_state(html: &str) -> Option<&str> {
    let marker = html.find("id=\"ImagesApp-")?;
    let tag_start = html[..marker].rfind("<div")?;
    let tag_end = tag_start + html[tag_start..].find('>')?;
    let tag = &html[tag_start..tag_end];
    let attr = tag.find("data-state=\"")? + "data-state=\"".len();
    let end = tag[attr..].find('"')?;
    Some(&tag[attr..attr + end])
}

/// Decodes the HTML entities that appear in the state attribute. One pass, so `&amp;quot;` stays `&quot;`.
#[expect(
    clippy::string_slice,
    reason = "every index is a find result, or one past an ASCII '&' or ';': a char boundary"
)]
fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let decoded =
            tail.find(';').filter(|&end| end <= 10).and_then(|end| decode_entity(&tail[1..end]).map(|c| (c, end)));
        match decoded {
            Some((c, end)) => {
                out.push(c);
                rest = &tail[end + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn decode_entity(name: &str) -> Option<char> {
    match name {
        "quot" => Some('"'),
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "apos" => Some('\''),
        _ => {
            let code = if let Some(hex) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
                u32::from_str_radix(hex, 16).ok()?
            } else {
                name.strip_prefix('#')?.parse().ok()?
            };
            char::from_u32(code)
        }
    }
}

/// Every group in `cbirOtherSizes`, largest first. Items without a readable size or URL are skipped.
pub fn parse_other_sizes(html: &str) -> Result<Vec<OtherSize>, YandexError> {
    let raw = extract_data_state(html).ok_or(YandexError::Unexpected("results page has no ImagesApp state"))?;
    let state: Value = serde_json::from_str(&decode_entities(raw))
        .map_err(|_| YandexError::Unexpected("results state is not JSON"))?;
    let Some(groups) = state.pointer("/initialState/cbirOtherSizes/items").and_then(Value::as_object) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for (group, list) in groups {
        for item in list.as_array().into_iter().flatten() {
            let (Some(text), Some(url)) = (item["text"].as_str(), item["url"].as_str()) else {
                continue;
            };
            let Some((width, height)) = parse_size(text) else {
                continue;
            };
            out.push(OtherSize { group: group.clone(), width, height, url: url.to_string() });
        }
    }
    out.sort_by_key(|s| std::cmp::Reverse(u64::from(s.width) * u64::from(s.height)));
    Ok(out)
}

/// `"714×320"` → (714, 320).
fn parse_size(text: &str) -> Option<(u32, u32)> {
    let (w, h) = text.split_once('×')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

/// One page of the similar endpoint. Errors with `Unexpected` when the JSON has no result list.
pub fn parse_similar_page(body: &str) -> Result<SimilarPage, YandexError> {
    let v: Value = serde_json::from_str(body).map_err(|_| YandexError::Unexpected("similar page is not JSON"))?;
    let serp = v
        .pointer("/blocks/1/params/adapterData/serpList")
        .ok_or(YandexError::Unexpected("similar page has no result list"))?;
    let mut items: Vec<SimilarItem> = serp
        .pointer("/items/entities")
        .and_then(Value::as_object)
        .map(|entities| entities.values().filter_map(similar_item).collect())
        .unwrap_or_default();
    items.sort_by_key(|i| i.pos);
    let has_next = serp.pointer("/navi/next").is_some_and(|n| n.is_string());
    Ok(SimilarPage { items, has_next })
}

fn similar_item(entity: &Value) -> Option<SimilarItem> {
    let pos = u32::try_from(entity["pos"].as_u64()?).ok()?;
    let image_url = query_param(entity["url"].as_str()?, "img_url")?;
    let width = entity["origWidth"].as_u64().or_else(|| entity["width"].as_u64()).unwrap_or(0);
    let height = entity["origHeight"].as_u64().or_else(|| entity["height"].as_u64()).unwrap_or(0);
    Some(SimilarItem {
        pos,
        width: u32::try_from(width).unwrap_or(0),
        height: u32::try_from(height).unwrap_or(0),
        image_url,
    })
}

/// The decoded value of `key` in a URL's query string.
fn query_param(url: &str, key: &str) -> Option<String> {
    let (_, query) = url.split_once('?')?;
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then(|| percent_decode(v))
    })
}

fn percent_decode(s: &str) -> String {
    let mut rest = s.as_bytes();
    let mut out = Vec::with_capacity(rest.len());
    while let [b, tail @ ..] = rest {
        if *b == b'%'
            && let [hi, lo, after @ ..] = tail
            && let Some(v) = std::str::from_utf8(&[*hi, *lo]).ok().and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            rest = after;
            continue;
        }
        out.push(*b);
        rest = tail;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A results page with the state attribute as Yandex writes it: JSON, HTML-escaped inside the attribute.
    fn results_html(items: &str) -> String {
        let state = format!(r#"{{"initialState":{{"cbirOtherSizes":{{"items":{{"small_dups":{items}}}}}}}}}"#);
        let escaped = state.replace('&', "&amp;").replace('"', "&quot;");
        format!(r#"<html><body><div class="Root" id="ImagesApp-Q1" data-state="{escaped}"></div></body></html>"#)
    }

    #[test]
    fn other_sizes_are_read_largest_first_and_entities_decoded() {
        let html = results_html(
            r#"[{"text":"714×320","url":"https://i.example/a.jpg?w=714&h=320"},{"text":"2560×1150","url":"https://i.example/b.jpg"},{"text":"bad","url":"https://i.example/c.jpg"}]"#,
        );
        let sizes = parse_other_sizes(&html).unwrap();
        assert_eq!(sizes.len(), 2, "the item with an unreadable size is skipped");
        assert_eq!((sizes[0].width, sizes[0].height), (2560, 1150));
        assert_eq!(sizes[1].url, "https://i.example/a.jpg?w=714&h=320");
        assert_eq!(sizes[1].group, "small_dups");
    }

    #[test]
    fn a_page_without_other_sizes_gives_an_empty_list() {
        let html = r#"<div id="ImagesApp-X" data-state="{&quot;initialState&quot;:{}}"></div>"#;
        assert!(parse_other_sizes(html).unwrap().is_empty());
    }

    #[test]
    fn a_page_without_state_is_an_error() {
        assert!(matches!(parse_other_sizes("<html></html>"), Err(YandexError::Unexpected(_))));
    }

    #[test]
    fn similar_page_items_come_from_the_img_url_parameter() {
        let body = r#"{"blocks":[{},{"params":{"adapterData":{"serpList":{
            "items":{"entities":{
                "a":{"pos":1,"image":"//thumbs.example/one.jpg","url":"/images/search?cbir_id=x&img_url=https%3A%2F%2Fa.example%2Fone.jpg&pos=1","origWidth":640,"origHeight":480},
                "b":{"pos":0,"url":"/images/search?img_url=https%3A%2F%2Fb.example%2Ftwo.png%3Fq%3D1","width":300,"height":200}
            }},
            "navi":{"next":"/images/search?p=1"}}}}}]}"#;
        let page = parse_similar_page(body).unwrap();
        assert!(page.has_next);
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.items[0].pos, 0, "sorted by position");
        assert_eq!(page.items[0].image_url, "https://b.example/two.png?q=1");
        assert_eq!((page.items[0].width, page.items[0].height), (300, 200));
        assert_eq!(page.items[1].image_url, "https://a.example/one.jpg");
    }

    #[test]
    fn similar_page_without_next_link_is_the_last_one() {
        let body = r#"{"blocks":[{},{"params":{"adapterData":{"serpList":{"items":{"entities":{}},"navi":{}}}}}]}"#;
        let page = parse_similar_page(body).unwrap();
        assert!(!page.has_next);
        assert!(page.items.is_empty());
    }

    #[test]
    fn similar_page_without_result_list_is_unexpected() {
        assert!(matches!(parse_similar_page(r#"{"error":"x"}"#), Err(YandexError::Unexpected(_))));
    }

    #[test]
    fn entity_decoding_keeps_double_escapes_intact() {
        assert_eq!(decode_entities("&amp;quot;"), "&quot;");
        assert_eq!(decode_entities("a &b c &#x27;d&#39;"), "a &b c 'd'");
        assert_eq!(decode_entities("&unknown; ok"), "&unknown; ok");
    }

    #[test]
    fn data_state_is_found_on_the_element_that_carries_it() {
        let html = r#"<div class="x"></div><div id="ImagesApp-A" data-state="one"></div><div data-state="two"></div>"#;
        assert_eq!(extract_data_state(html), Some("one"));
    }

    #[test]
    fn size_text_parses_with_the_multiplication_sign() {
        assert_eq!(parse_size("714×320"), Some((714, 320)));
        assert_eq!(parse_size("714x320"), None);
    }

    #[test]
    fn a_later_page_that_fails_stops_the_list_with_its_reason() {
        let item = |pos: u32| SimilarItem { pos, width: 1, height: 1, image_url: format!("u{pos}") };
        let page = |n: u32, has_next: bool| -> Result<SimilarPage, YandexError> {
            Ok(SimilarPage { items: vec![item(n)], has_next })
        };
        // Pages 0 and 1 are read; page 2 is a captcha: the list keeps what it has, and says why it stopped.
        let got = read_similar_pages(5, |p| match p {
            0 | 1 => page(p, true),
            _ => Err(YandexError::Captcha),
        })
        .unwrap();
        assert_eq!(got.items.iter().map(|i| i.pos).collect::<Vec<_>>(), [0, 1]);
        assert!(matches!(got.stopped, Some(YandexError::Captcha)));
        // A failure on the first page is the error itself.
        assert!(matches!(read_similar_pages(5, |_| Err(YandexError::Captcha)), Err(YandexError::Captcha)));
        // A list that ends by itself (no next page) has no reason.
        let got = read_similar_pages(5, |p| page(p, p < 1)).unwrap();
        assert_eq!(got.items.len(), 2);
        assert!(got.stopped.is_none());
    }

    #[test]
    fn percent_decoding_handles_truncated_escapes() {
        assert_eq!(percent_decode("a%2Fb%3"), "a/b%3");
        assert_eq!(percent_decode("%E2%9C%93"), "✓");
    }
}
