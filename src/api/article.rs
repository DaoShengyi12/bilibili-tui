//! Article response models and normalization for terminal rendering.

use scraper::{Html, Selector};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct ArticleData {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default, deserialize_with = "deserialize_content")]
    pub content: String,
    #[serde(default, rename = "type")]
    pub article_type: i32,
    #[serde(default)]
    pub author: Option<ArticleAuthor>,
    #[serde(default)]
    pub publish_time: i64,
    #[serde(default)]
    pub image_urls: Vec<String>,
    #[serde(default)]
    pub origin_image_urls: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ArticleAuthor {
    #[serde(default)]
    pub mid: i64,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArticleDocument {
    pub body: String,
    pub image_urls: Vec<String>,
    pub blocks: Vec<ArticleBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArticleBlock {
    Text(String),
    Image { url: String, alt: String },
    Embedded(String),
}

fn deserialize_content<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Null => Ok(String::new()),
        serde_json::Value::String(content) => Ok(content),
        value => serde_json::to_string(&value).map_err(serde::de::Error::custom),
    }
}

impl ArticleData {
    pub fn document(&self) -> ArticleDocument {
        let mut document = if self.article_type == 3 || self.content.trim_start().starts_with('{') {
            document_from_json(&self.content)
        } else {
            document_from_html(&self.content)
        };

        for url in self.image_urls.iter().chain(self.origin_image_urls.iter()) {
            let url = normalized_url(url);
            if !url.is_empty() && !document.image_urls.contains(&url) {
                document.image_urls.push(url.clone());
                document.blocks.push(ArticleBlock::Image {
                    url,
                    alt: "文章图片".to_string(),
                });
            }
        }
        if document.blocks.is_empty() && !self.summary.trim().is_empty() {
            document
                .blocks
                .push(ArticleBlock::Text(self.summary.clone()));
        }
        document.body = document_body(&document.blocks);
        document
    }
}

fn document_from_html(content: &str) -> ArticleDocument {
    let html = Html::parse_fragment(content);
    let blocks = Selector::parse("h1,h2,h3,h4,p,blockquote,li,pre,figure,img").expect("selector");
    let images = Selector::parse("img").expect("selector");
    let captions = Selector::parse("figcaption").expect("selector");
    let links = Selector::parse("a[href]").expect("selector");
    let mut document_blocks = Vec::new();
    let mut image_urls = Vec::new();

    for element in html.select(&blocks) {
        let name = element.value().name();
        if name == "figure" {
            if let Some(image) = element.select(&images).next() {
                if let Some(placeholder) = embedded_placeholder(&image) {
                    document_blocks.push(ArticleBlock::Embedded(placeholder));
                } else if let Some(url) = image_url(&image) {
                    push_unique_url(&mut image_urls, &url);
                    document_blocks.push(ArticleBlock::Image {
                        url: normalized_url(&url),
                        alt: image.value().attr("alt").unwrap_or("文章图片").to_string(),
                    });
                }
            }
            if let Some(caption) = element.select(&captions).next() {
                let caption = element_text(&caption);
                if !caption.is_empty() {
                    document_blocks.push(ArticleBlock::Text(caption));
                }
            }
            continue;
        }
        if name == "img" {
            if has_ancestor(&element, &["figure"]) {
                continue;
            }
            if let Some(placeholder) = embedded_placeholder(&element) {
                document_blocks.push(ArticleBlock::Embedded(placeholder));
            } else if let Some(url) = image_url(&element) {
                push_unique_url(&mut image_urls, &url);
                document_blocks.push(ArticleBlock::Image {
                    url: normalized_url(&url),
                    alt: element
                        .value()
                        .attr("alt")
                        .unwrap_or("文章图片")
                        .to_string(),
                });
            }
            continue;
        }

        // The outermost block already includes the text of nested blocks such
        // as `<blockquote><p>`, and figures render their own captions.
        if has_ancestor(&element, &TEXT_BLOCKS) || has_ancestor(&element, &["figure"]) {
            continue;
        }
        let text = element_text(&element);
        let text = text.as_str();
        if text.is_empty() {
            continue;
        }
        let mut line = match name {
            "h1" | "h2" | "h3" | "h4" => format!("# {text}"),
            "blockquote" => format!("> {text}"),
            "li" => format!("• {text}"),
            "pre" => format!("```\n{text}\n```"),
            _ => text.to_string(),
        };
        for link in element.select(&links) {
            if let Some(href) = link.value().attr("href")
                && !href.is_empty()
            {
                line.push_str(&format!(" ({href})"));
            }
        }
        document_blocks.push(ArticleBlock::Text(line));
    }

    if document_blocks.is_empty() {
        let text = html.root_element().text().collect::<Vec<_>>().join("");
        if !text.trim().is_empty() {
            document_blocks.push(ArticleBlock::Text(text.trim().to_string()));
        }
    }

    ArticleDocument {
        body: document_body(&document_blocks),
        image_urls,
        blocks: document_blocks,
    }
}

const TEXT_BLOCKS: [&str; 8] = ["h1", "h2", "h3", "h4", "p", "blockquote", "li", "pre"];

fn has_ancestor(element: &scraper::ElementRef<'_>, names: &[&str]) -> bool {
    element.ancestors().any(|node| {
        scraper::ElementRef::wrap(node).is_some_and(|parent| names.contains(&parent.value().name()))
    })
}

fn element_text(element: &scraper::ElementRef<'_>) -> String {
    element.text().collect::<String>().trim().to_string()
}

fn document_from_json(content: &str) -> ArticleDocument {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(content) else {
        let blocks = vec![ArticleBlock::Text(content.to_string())];
        return ArticleDocument {
            body: content.to_string(),
            image_urls: Vec::new(),
            blocks,
        };
    };
    let mut blocks = Vec::new();
    let mut image_urls = Vec::new();
    for operation in value
        .get("ops")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(insert) = operation.get("insert") else {
            continue;
        };
        if let Some(text) = insert.as_str() {
            push_text_block(&mut blocks, text);
            continue;
        }
        let Some(object) = insert.as_object() else {
            continue;
        };
        if let Some(image) = object.get("native-image") {
            if let Some(url) = image.get("url").and_then(serde_json::Value::as_str) {
                push_unique_url(&mut image_urls, url);
                blocks.push(ArticleBlock::Image {
                    url: normalized_url(url),
                    alt: image
                        .get("alt")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("文章图片")
                        .to_string(),
                });
            }
        } else if let Some(card) = object.get("video-card") {
            push_json_card(&mut blocks, "视频", card);
        } else if let Some(card) = object.get("article-card") {
            push_json_card(&mut blocks, "文章", card);
        } else if let Some(card) = object.get("live-card") {
            push_json_card(&mut blocks, "直播", card);
        } else if let Some(card) = object.get("vote-card") {
            push_json_card(&mut blocks, "投票", card);
        } else if object.contains_key("cut-off") {
            blocks.push(ArticleBlock::Embedded("────────".to_string()));
        }
    }
    ArticleDocument {
        body: document_body(&blocks),
        image_urls,
        blocks,
    }
}

fn push_json_card(blocks: &mut Vec<ArticleBlock>, label: &str, card: &serde_json::Value) {
    let id = card
        .get("id")
        .and_then(|value| {
            value
                .as_str()
                .map(ToOwned::to_owned)
                .or_else(|| value.as_i64().map(|v| v.to_string()))
        })
        .unwrap_or_default();
    blocks.push(ArticleBlock::Embedded(format!("[{label} {id}]")));
}

fn push_text_block(blocks: &mut Vec<ArticleBlock>, text: &str) {
    if let Some(ArticleBlock::Text(previous)) = blocks.last_mut() {
        previous.push_str(text);
    } else {
        blocks.push(ArticleBlock::Text(text.to_string()));
    }
}

fn document_body(blocks: &[ArticleBlock]) -> String {
    let mut image_index = 0;
    blocks
        .iter()
        .map(|block| match block {
            ArticleBlock::Text(text) => text.trim().to_string(),
            ArticleBlock::Image { .. } => {
                image_index += 1;
                format!("[图片 {image_index}]")
            }
            ArticleBlock::Embedded(text) => text.clone(),
        })
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn embedded_placeholder(image: &scraper::ElementRef<'_>) -> Option<String> {
    let classes = image.value().attr("class").unwrap_or_default();
    let id = image
        .value()
        .attr("aid")
        .or_else(|| image.value().attr("data-aid"))
        .unwrap_or_default();
    [
        ("video-card", "视频"),
        ("article-card", "文章"),
        ("fanju-card", "番剧"),
        ("live-card", "直播"),
        ("music-card", "音乐"),
        ("vote-card", "投票"),
    ]
    .into_iter()
    .find_map(|(class, label)| classes.contains(class).then(|| format!("[{label} {id}]")))
}

fn image_url(image: &scraper::ElementRef<'_>) -> Option<String> {
    image
        .value()
        .attr("data-src")
        .or_else(|| image.value().attr("src"))
        .filter(|url| !url.is_empty())
        .map(str::to_owned)
}

fn push_unique_url(urls: &mut Vec<String>, url: &str) {
    let normalized = normalized_url(url);
    if !normalized.is_empty() && !urls.contains(&normalized) {
        urls.push(normalized);
    }
}

fn normalized_url(url: &str) -> String {
    if url.starts_with("//") {
        format!("https:{url}")
    } else {
        url.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_article_extracts_structure_cards_and_images() {
        let article = ArticleData {
            id: 1,
            title: "title".into(),
            summary: String::new(),
            content: r#"<h1>标题</h1><p>A &amp; B <a href="https://example.test">链接</a></p><figure><img data-src="//i.test/a.jpg"></figure><figure><img class="video-card" aid="2"></figure>"#.into(),
            article_type: 0,
            author: None,
            publish_time: 0,
            image_urls: Vec::new(),
            origin_image_urls: Vec::new(),
        };
        let document = article.document();
        assert!(document.body.contains("# 标题"));
        assert!(document.body.contains("A & B"));
        assert!(document.body.contains("https://example.test"));
        assert!(document.body.contains("[视频 2]"));
        assert_eq!(document.image_urls, vec!["https://i.test/a.jpg"]);
        assert!(matches!(document.blocks[0], ArticleBlock::Text(_)));
        assert!(matches!(document.blocks[1], ArticleBlock::Text(_)));
        assert!(matches!(
            &document.blocks[2],
            ArticleBlock::Image { url, .. } if url == "https://i.test/a.jpg"
        ));
        assert!(
            matches!(
                &document.blocks[3],
                ArticleBlock::Embedded(text) if text == "[视频 2]"
            ),
            "{:?}",
            document.blocks
        );
    }

    #[test]
    fn html_nested_blocks_render_once_and_keep_figure_captions() {
        let document = document_from_html(
            r#"<blockquote><p>引用</p></blockquote><ul><li><p>条目</p></li></ul><figure><img src="//i.test/a.jpg"><figcaption><p>图注</p></figcaption></figure>"#,
        );
        assert_eq!(document.body.matches("引用").count(), 1);
        assert!(document.body.contains("> 引用"));
        assert_eq!(document.body.matches("条目").count(), 1);
        assert!(document.body.contains("• 条目"));
        assert_eq!(document.body.matches("图注").count(), 1);
        assert!(matches!(
            &document.blocks[3],
            ArticleBlock::Text(text) if text == "图注"
        ));
    }

    #[test]
    fn json_article_extracts_text_and_native_images() {
        let document = document_from_json(
            r#"{"ops":[{"insert":"hello\n"},{"insert":{"native-image":{"url":"https://i.test/a.jpg"}}},{"insert":{"article-card":{"id":"cv7"}}}]}"#,
        );
        assert!(document.body.contains("hello"));
        assert!(document.body.contains("[图片 1]"));
        assert!(document.body.contains("[文章 cv7]"));
        assert_eq!(document.image_urls.len(), 1);
        assert!(matches!(document.blocks[0], ArticleBlock::Text(_)));
        assert!(matches!(document.blocks[1], ArticleBlock::Image { .. }));
        assert!(matches!(document.blocks[2], ArticleBlock::Embedded(_)));
    }

    #[test]
    fn json_content_object_deserializes_without_losing_operations() {
        let article: ArticleData = serde_json::from_value(serde_json::json!({
            "type": 3,
            "content": {"ops": [{"insert": "text"}]}
        }))
        .unwrap();
        assert_eq!(article.document().body, "text");
    }
}
