use chrono::{NaiveDate, NaiveDateTime};
use frontmatter_gen::{Frontmatter, Value};
use log::error;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process;
use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Clone, Serialize)]
pub enum Kind {
    Tag,
    Archive,
    Author,
    Stream,
}

#[allow(clippy::module_name_repetitions)]
#[derive(Debug, Clone, Serialize)]
pub struct GroupedContent {
    pub kind: Kind,
    pub map: HashMap<String, Vec<Content>>,
}

impl GroupedContent {
    pub fn new(kind: Kind) -> Self {
        Self {
            kind,
            map: HashMap::new(),
        }
    }

    pub fn entry(&mut self, key: String) -> Entry<String, Vec<Content>> {
        self.map.entry(key)
    }

    /// Sort tag map by number of contents
    /// Sort archive map by date
    /// Sort author map by author name
    /// Sort stream map by stream name
    pub fn iter(&self) -> impl Iterator<Item = (&String, Vec<Content>)> {
        let mut vec = Vec::new();
        match self.kind {
            Kind::Tag => {
                for (tag, contents) in &self.map {
                    let mut contents = contents.clone();
                    contents.sort_by(|a, b| b.date.cmp(&a.date));
                    vec.push((tag, contents));
                }
                vec.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
            }
            Kind::Archive => {
                for (text, contents) in &self.map {
                    let mut contents = contents.clone();
                    contents.sort_by(|a, b| b.date.cmp(&a.date));
                    vec.push((text, contents));
                }
                vec.sort_by(|a, b| b.0.cmp(a.0));
            }
            Kind::Author | Kind::Stream => {
                for (text, contents) in &self.map {
                    let mut contents = contents.clone();
                    contents.sort_by(|a, b| b.date.cmp(&a.date));
                    vec.push((text, contents));
                }
                vec.sort_by(|a, b| a.0.cmp(b.0));
            }
        }
        vec.into_iter()
    }
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct Content {
    pub title: String,
    pub description: Option<String>,
    pub slug: String,
    pub html: String,
    pub tags: Vec<String>,
    pub date: Option<NaiveDateTime>,
    pub extra: Option<Value>,
    pub links_to: Option<Vec<String>>,
    pub back_links: Vec<Self>,
    pub card_image: Option<String>,
    pub banner_image: Option<String>,
    pub authors: Vec<String>,
    pub stream: Option<String>,
}

/// Try to get the title from the frontmatter
/// If not found, get the first line of the markdown without the leading '#'
/// If no lines are found, return an empty string
/// return (title, markdown without title)
pub fn get_title<'a>(frontmatter: &'a Frontmatter, markdown: &'a str) -> (String, String) {
    let title = match frontmatter.get("title") {
        Some(Value::String(t)) => t.to_string(),
        _ => markdown
            .lines()
            .find(|line| !line.is_empty())
            .unwrap_or("")
            .trim_start_matches('#')
            .trim()
            .to_string(),
    };
    let markdown = markdown
        .lines()
        .skip_while(|line| {
            line.trim().is_empty()
                || line.trim().starts_with('#') && line.trim_start_matches('#').trim() == title
                || line.trim() == title
        })
        .collect::<Vec<&str>>()
        .join("\n");
    (title, markdown)
}

pub fn get_description(frontmatter: &Frontmatter) -> Option<String> {
    if let Some(description) = frontmatter.get("description") {
        return Some(description.to_string());
    }
    None
}

/// Try to get the slug from the frontmatter
/// If not found, get the title from the frontmatter
/// If not found, get the filename without the date
/// If a date is found in the filename, remove it from the slug
/// If a stream is not the default `index`, prepend it to the slug
/// return the slug
pub fn get_slug<'a>(frontmatter: &'a Frontmatter, path: &'a Path) -> String {
    let stream = get_stream(frontmatter).unwrap();
    let mut final_slug: String;

    if let Some(slug) = frontmatter.get("slug") {
        final_slug = slugify(&slug.to_string());
    } else if let Some(title) = frontmatter.get("title") {
        final_slug = slugify(&title.to_string());
    } else {
        final_slug = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap()
            .to_string();
        if let Some(date) = extract_date_from_filename(path) {
            final_slug = final_slug.replace(&format!("{}-", date.date()), "");
        }
    }

    if stream != "index" {
        final_slug = format!("{stream}-{final_slug}");
    }

    final_slug
}

/// Capture `stream` from frontmatter
/// If not defined return "index" as default
#[allow(clippy::unnecessary_wraps)]
pub fn get_stream(frontmatter: &Frontmatter) -> Option<String> {
    if let Some(stream) = frontmatter.get("stream") {
        return Some(stream.as_str().unwrap().trim_matches('"').to_string());
    }
    Some("index".to_string())
}

pub fn get_tags(frontmatter: &Frontmatter) -> Vec<String> {
    let tags: Vec<String> = match frontmatter.get("tags") {
        Some(Value::Array(tags)) => tags
            .iter()
            .map(Value::to_string)
            .map(|t| t.trim_matches('"').to_string())
            .collect(),
        Some(Value::String(tags)) => tags.split(',').map(str::trim).map(String::from).collect(),
        _ => Vec::new(),
    };
    tags
}

pub fn get_authors(frontmatter: &Frontmatter) -> Vec<String> {
    let authors: Vec<String> = match frontmatter.get("authors") {
        Some(Value::Array(authors)) => authors
            .iter()
            .map(Value::to_string)
            .map(|t| t.trim_matches('"').to_string())
            .collect(),
        Some(Value::String(authors)) => authors
            .split(',')
            .map(str::trim)
            .map(String::from)
            .collect(),
        _ => Vec::new(),
    };
    authors
}

/// Tries to get `date` from the front-matter metadata, else from filename
/// Input examples:
///   frontmatter = Frontmatter {date: Value("2024-10-10")}
///   path = "2024-01-01-myfile.md"
pub fn get_date(frontmatter: &Frontmatter, path: &Path) -> Option<NaiveDateTime> {
    if let Some(input) = frontmatter.get("date").and_then(|v| v.as_str()) {
        match try_to_parse_date(input) {
            Ok(date) => return Some(date),
            Err(e) => {
                error!(
                    "ERROR: Invalid date format {} when parsing {}, {}",
                    input,
                    path.display(),
                    e.to_string()
                );
                process::exit(1);
            }
        }
    }
    extract_date_from_filename(path)
}

/// Tries to parse 3 different date formats or return Error.
/// input: "2024-01-01 15:40:56" | "2024-01-01 15:40" | "2024-01-01"
fn try_to_parse_date(input: &str) -> Result<NaiveDateTime, chrono::ParseError> {
    NaiveDateTime::parse_from_str(input, "%Y-%m-%d %H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(input, "%Y-%m-%d %H:%M"))
        .or_else(|_| {
            NaiveDate::parse_from_str(input, "%Y-%m-%d").map(|d| d.and_hms_opt(0, 0, 0).unwrap())
        })
}

/// Use regex to extract date from filename `2024-01-01-myfile.md`
fn extract_date_from_filename(path: &Path) -> Option<NaiveDateTime> {
    let date_re = Regex::new(r"\d{4}-\d{2}-\d{2}").unwrap();
    date_re
        .find(path.to_str().unwrap())
        .and_then(|m| NaiveDate::parse_from_str(m.as_str(), "%Y-%m-%d").ok())
        .and_then(|dt| dt.and_hms_opt(0, 0, 0))
}

pub fn check_for_duplicate_slugs(contents: &Vec<&Content>) -> Result<(), String> {
    let mut seen = HashSet::new();

    for content in contents {
        if !seen.insert(&content.slug) {
            return Err(content.slug.clone());
        }
    }

    Ok(())
}

pub fn slugify(text: &str) -> String {
    let normalized = text.nfd().collect::<String>().to_lowercase();
    let re = Regex::new(r"[^a-z0-9]+").unwrap();
    let slug = re.replace_all(&normalized, "-");
    slug.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_title_from_frontmatter() {
        let mut frontmatter = Frontmatter::new();
        frontmatter.insert("title".to_string(), Value::String("Test Title".to_string()));
        let markdown = "# HTML Title";

        let (title, markdown) = get_title(&frontmatter, markdown);
        assert_eq!(title, "Test Title");
        assert!(markdown.contains("HTML Title"));
    }

    #[test]
    fn test_get_title_from_html() {
        let frontmatter = Frontmatter::new();
        let markdown = "# HTML Title";

        let (title, markdown) = get_title(&frontmatter, markdown);
        assert_eq!(title, "HTML Title");
        assert!(!markdown.contains("HTML Title"));
    }

    #[test]
    fn test_get_title_from_html_with_no_title_tag() {
        let frontmatter = Frontmatter::new();
        let markdown = "title here";

        let (title, markdown) = get_title(&frontmatter, markdown);
        assert_eq!(title, "title here");
        assert!(!markdown.contains("title here"));
    }

    #[test]
    fn test_get_title_from_html_with_multiple_lines() {
        let frontmatter = Frontmatter::new();
        let markdown = "
# First Title
Second Title
        ";

        let (title, markdown) = get_title(&frontmatter, markdown);
        assert_eq!(title, "First Title");
        assert!(!markdown.contains("First Title"));
        assert!(markdown.contains("Second Title"));
    }

    #[test]
    fn test_get_description_from_frontmatter() {
        let mut frontmatter = Frontmatter::new();
        frontmatter.insert(
            "description".to_string(),
            Value::String("Test Description".to_string()),
        );

        let description = get_description(&frontmatter);
        assert_eq!(description, Some("\"Test Description\"".to_string()));
    }

    #[test]
    fn test_get_description_from_empty_frontmatter() {
        let frontmatter = Frontmatter::new();

        let description = get_description(&frontmatter);
        assert_eq!(description, None);
    }

    #[test]
    fn test_get_slug_from_frontmatter() {
        let mut frontmatter = Frontmatter::new();
        frontmatter.insert("slug".to_string(), Value::String("test-slug".to_string()));
        let path = Path::new("2024-01-01-myfile.md");

        let slug = get_slug(&frontmatter, path);
        assert_eq!(slug, "test-slug");
    }

    #[test]
    fn test_get_slug_from_title() {
        let mut frontmatter = Frontmatter::new();
        frontmatter.insert("title".to_string(), Value::String("Test Title".to_string()));
        let path = Path::new("2024-01-01-myfile.md");

        let slug = get_slug(&frontmatter, path);
        assert_eq!(slug, "test-title");
    }

    #[test]
    fn test_get_slug_from_filename() {
        let frontmatter = Frontmatter::new();
        let path = Path::new("2024-01-01-myfile.md");

        let slug = get_slug(&frontmatter, path);
        assert_eq!(slug, "myfile");
    }

    #[test]
    fn test_get_slug_from_filename_without_date() {
        let frontmatter = Frontmatter::new();
        let path = Path::new("myfile.md");

        let slug = get_slug(&frontmatter, path);
        assert_eq!(slug, "myfile");
    }

    #[test]
    fn test_get_slug_with_special_characters() {
        let mut frontmatter = Frontmatter::new();
        frontmatter.insert(
            "title".to_string(),
            Value::String("Test Title with Special Characters!@#".to_string()),
        );
        let path = Path::new("2024-01-01-myfile.md");

        let slug = get_slug(&frontmatter, path);
        assert_eq!(slug, "test-title-with-special-characters");
    }

    #[test]
    fn test_get_tags_from_frontmatter_array() {
        let mut frontmatter = Frontmatter::new();
        frontmatter.insert(
            "tags".to_string(),
            Value::Array(vec![
                Value::String("tag1".to_string()),
                Value::String("tag2".to_string()),
            ]),
        );

        let tags = get_tags(&frontmatter);
        assert_eq!(tags, vec!["tag1", "tag2"]);
    }

    #[test]
    fn test_get_tags_from_frontmatter_string() {
        let mut frontmatter = Frontmatter::new();
        frontmatter.insert("tags".to_string(), Value::String("tag1, tag2".to_string()));

        let tags = get_tags(&frontmatter);
        assert_eq!(tags, vec!["tag1", "tag2"]);
    }

    #[test]
    fn test_get_tags_with_no_tags() {
        let frontmatter = Frontmatter::new();

        let tags = get_tags(&frontmatter);
        assert!(tags.is_empty());
    }

    #[test]
    fn test_get_date_from_frontmatter() {
        let mut frontmatter = Frontmatter::new();
        frontmatter.insert(
            "date".to_string(),
            Value::String("2024-01-01 15:40:56".to_string()),
        );
        let path = Path::new("myfile.md");

        let date = get_date(&frontmatter, path).unwrap();
        assert_eq!(
            date,
            NaiveDate::from_ymd_opt(2024, 1, 1)
                .unwrap()
                .and_hms_opt(15, 40, 56)
                .unwrap()
        );
    }

    #[test]
    fn test_get_date_from_frontmatter_without_time() {
        let mut frontmatter = Frontmatter::new();
        frontmatter.insert("date".to_string(), Value::String("2024-01-01".to_string()));
        let path = Path::new("myfile.md");

        let date = get_date(&frontmatter, path).unwrap();
        assert_eq!(
            date,
            NaiveDate::from_ymd_opt(2024, 1, 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap()
        );
    }

    #[test]
    fn test_get_date_from_filename() {
        let frontmatter = Frontmatter::new();
        let path = Path::new("2024-01-01-myfile.md");

        let date = get_date(&frontmatter, path).unwrap();
        assert_eq!(
            date,
            NaiveDate::from_ymd_opt(2024, 1, 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap()
        );
    }

    #[test]
    fn test_get_date_no_date() {
        let frontmatter = Frontmatter::new();
        let path = Path::new("myfile.md");

        let date = get_date(&frontmatter, path);
        assert!(date.is_none());
    }

    #[test]
    fn test_slugify_simple_text() {
        let text = "Simple Text";
        let slug = slugify(text);
        assert_eq!(slug, "simple-text");
    }

    #[test]
    fn test_slugify_with_special_characters() {
        let text = "Text with Special Characters!@#";
        let slug = slugify(text);
        assert_eq!(slug, "text-with-special-characters");
    }

    #[test]
    fn test_slugify_with_accents() {
        let text = "Téxt wíth Áccénts";
        let slug = slugify(text);
        assert_eq!(slug, "te-xt-wi-th-a-cce-nts");
    }

    #[test]
    fn test_slugify_with_multiple_spaces() {
        let text = "Text    with    multiple    spaces";
        let slug = slugify(text);
        assert_eq!(slug, "text-with-multiple-spaces");
    }

    #[test]
    fn test_slugify_with_underscores() {
        let text = "Text_with_underscores";
        let slug = slugify(text);
        assert_eq!(slug, "text-with-underscores");
    }

    #[test]
    fn test_slugify_with_numbers() {
        let text = "Text with numbers 123";
        let slug = slugify(text);
        assert_eq!(slug, "text-with-numbers-123");
    }

    #[test]
    fn test_slugify_empty_string() {
        let text = "";
        let slug = slugify(text);
        assert_eq!(slug, "");
    }

    #[test]
    fn test_check_for_duplicate_slugs_no_duplicates() {
        let content1 = Content {
            title: "Title 1".to_string(),
            description: None,
            slug: "slug-1".to_string(),
            html: String::new(),
            tags: vec![],
            date: None,
            extra: None,
            links_to: None,
            back_links: vec![],
            card_image: None,
            banner_image: None,
            authors: vec![],
            stream: None,
        };
        let content2 = Content {
            title: "Title 2".to_string(),
            description: None,
            slug: "slug-2".to_string(),
            html: String::new(),
            tags: vec![],
            date: None,
            extra: None,
            links_to: None,
            back_links: vec![],
            card_image: None,
            banner_image: None,
            authors: vec![],
            stream: None,
        };
        let contents = vec![&content1, &content2];
        let result = check_for_duplicate_slugs(&contents);
        assert!(result.is_ok());
    }

    #[test]
    fn test_check_for_duplicate_slugs_with_duplicates() {
        let content1 = Content {
            title: "Title 1".to_string(),
            description: None,
            slug: "duplicate-slug".to_string(),
            html: String::new(),
            tags: vec![],
            date: None,
            extra: None,
            links_to: None,
            back_links: vec![],
            card_image: None,
            banner_image: None,
            authors: vec![],
            stream: None,
        };
        let content2 = Content {
            title: "Title 2".to_string(),
            description: None,
            slug: "duplicate-slug".to_string(),
            html: String::new(),
            tags: vec![],
            date: None,
            extra: None,
            links_to: None,
            back_links: vec![],
            card_image: None,
            banner_image: None,
            authors: vec![],
            stream: None,
        };
        let contents = vec![&content1, &content2];

        let result = check_for_duplicate_slugs(&contents);
        assert!(result.is_err());
        assert_eq!(result.err().unwrap(), "duplicate-slug".to_string());
    }

    #[test]
    fn test_check_for_duplicate_slugs_empty_list() {
        let contents: Vec<&Content> = vec![];

        let result = check_for_duplicate_slugs(&contents);
        assert!(result.is_ok());
    }

    #[test]
    fn test_extract_date_from_filename_valid_date() {
        let path = Path::new("2024-01-01-myfile.md");
        let date = extract_date_from_filename(path).unwrap();
        assert_eq!(
            date,
            NaiveDate::from_ymd_opt(2024, 1, 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap()
        );
    }

    #[test]
    fn test_extract_date_from_filename_invalid_date() {
        let path = Path::new("not-a-date-myfile.md");
        let date = extract_date_from_filename(path);
        assert!(date.is_none());
    }

    #[test]
    fn test_extract_date_from_filename_empty() {
        let path = Path::new("");
        let date = extract_date_from_filename(path);
        assert!(date.is_none());
    }

    #[test]
    fn test_extract_date_from_filename_with_time() {
        let path = Path::new("2024-01-01-15-30-myfile.md");
        let date = extract_date_from_filename(path).unwrap();
        assert_eq!(
            date,
            NaiveDate::from_ymd_opt(2024, 1, 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap()
        );
    }

    #[test]
    fn test_extract_date_from_filename_with_multiple_dates() {
        let path = Path::new("2024-01-01-2025-02-02-myfile.md");
        let date = extract_date_from_filename(path).unwrap();
        assert_eq!(
            date,
            NaiveDate::from_ymd_opt(2024, 1, 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap()
        );
    }
}
