use std::ops::Range;

use pulldown_cmark::{Event, Parser, Tag};
use regex::Regex;

/// Returns the part of a GitHub markdown body that GitHub shows before any
/// clicks: HTML comments are dropped and a collapsed `<details>` block is
/// reduced to its summary.
pub fn visible_text(body: &str) -> String {
    let body = body.replace("\r\n", "\n");
    let body = body.as_str();
    let mut out = String::new();
    let mut stack: Vec<Details> = Vec::new();
    let mut in_summary = false;
    let mut pos = 0;

    for (range, token) in html_tokens(body) {
        if is_visible(&stack, in_summary) {
            push_joined(&mut out, &body[pos..range.start]);
        }
        pos = range.end;

        match token {
            Token::Comment => {}
            Token::DetailsOpen { open } => stack.push(Details {
                open,
                summarized: false,
            }),
            Token::DetailsClose => {
                // An unclosed summary ends with its block, as in browsers.
                in_summary = false;
                close_details(&mut out, &mut stack);
            }
            Token::SummaryOpen => {
                let outer_visible = is_visible(&stack[..stack.len().saturating_sub(1)], false);
                // Only the first summary labels the block; later ones are content.
                if let Some(details) = stack.last_mut().filter(|d| !d.summarized) {
                    details.summarized = true;
                    in_summary = true;
                    if !details.open && outer_visible {
                        out.push_str("▸ ");
                    }
                }
            }
            Token::SummaryClose => in_summary = false,
        }
    }

    if is_visible(&stack, in_summary) {
        push_joined(&mut out, &body[pos..]);
    }
    while !stack.is_empty() {
        close_details(&mut out, &mut stack);
    }

    out.trim().to_string()
}

struct Details {
    open: bool,
    summarized: bool,
}

enum Token {
    Comment,
    DetailsOpen { open: bool },
    DetailsClose,
    SummaryOpen,
    SummaryClose,
}

// While in a summary, the innermost block's own state doesn't hide it.
fn is_visible(stack: &[Details], in_summary: bool) -> bool {
    match stack.split_last() {
        None => true,
        Some((top, outer)) => (top.open || in_summary) && outer.iter().all(|d| d.open),
    }
}

fn close_details(out: &mut String, stack: &mut Vec<Details>) {
    if let Some(details) = stack.pop() {
        // GitHub labels a block with no summary "Details".
        if !details.summarized && !details.open && is_visible(stack, false) {
            push_joined(out, "▸ Details");
        }
    }
}

// Removed tags leave their surrounding blank lines behind; keep at most one
// blank line at each seam.
fn push_joined(out: &mut String, segment: &str) {
    let trailing = out.len() - out.trim_end_matches('\n').len();
    let leading = segment.len() - segment.trim_start_matches('\n').len();
    let keep = leading.min(2usize.saturating_sub(trailing));
    out.push_str(&segment[leading - keep..]);
}

// Tags count only where the markdown parser sees raw HTML, so code blocks and
// code spans keep them as literal text.
fn html_tokens(body: &str) -> Vec<(Range<usize>, Token)> {
    // An unterminated comment runs to the end of its HTML block, which
    // CommonMark extends to the end of the body.
    let tag = Regex::new(r"(?is)<!--.*?(?:-->|\z)|<(/?)(details|summary)\b([^>]*)>").unwrap();
    let quoted = Regex::new(r#""[^"]*"|'[^']*'"#).unwrap();
    let open_attr = Regex::new(r"(?i)(?:^|\s)open(?:\s|=|$)").unwrap();

    let html_ranges = Parser::new(body)
        .into_offset_iter()
        .filter(|(event, _)| matches!(event, Event::Start(Tag::HtmlBlock) | Event::InlineHtml(_)))
        .map(|(_, range)| range);

    let mut tokens = Vec::new();
    for html in html_ranges {
        for caps in tag.captures_iter(&body[html.clone()]) {
            let m = caps.get(0).unwrap();
            let range = html.start + m.start()..html.start + m.end();
            let closing = caps.get(1).is_some_and(|c| c.as_str() == "/");
            let token = match caps.get(2).map(|n| n.as_str().to_lowercase()) {
                None => Token::Comment,
                Some(name) if name == "details" && closing => Token::DetailsClose,
                Some(name) if name == "details" => Token::DetailsOpen {
                    open: open_attr.is_match(&quoted.replace_all(&caps[3], "")),
                },
                Some(_) if closing => Token::SummaryClose,
                Some(_) => Token::SummaryOpen,
            };
            tokens.push((range, token));
        }
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plain_text_unchanged() {
        assert_eq!("Looks *good* to me", visible_text("  Looks *good* to me\n"));
    }

    #[test]
    fn test_details_collapsed_to_summary() {
        let body = "Top line\n\n<details><summary>More notes</summary>\n\n- one\n- two\n\n</details>\n\nBottom line";
        assert_eq!(
            "Top line\n\n▸ More notes\n\nBottom line",
            visible_text(body)
        );
    }

    #[test]
    fn test_details_summary_on_own_lines() {
        let body = "<details>\n<summary>More notes</summary>\n\nhidden\n</details>";
        assert_eq!("▸ More notes", visible_text(body));
    }

    #[test]
    fn test_details_without_summary() {
        let body = "Top\n\n<details>\n\nhidden\n\n</details>";
        assert_eq!("Top\n\n▸ Details", visible_text(body));
    }

    #[test]
    fn test_details_open_keeps_contents() {
        let body = "<details open><summary>Notes</summary>\n\nshown\n\n</details>";
        assert_eq!("Notes\n\nshown", visible_text(body));
    }

    #[test]
    fn test_details_case_insensitive() {
        let body = "<DETAILS><Summary>Notes</Summary>\n\nhidden\n\n</Details>";
        assert_eq!("▸ Notes", visible_text(body));
    }

    #[test]
    fn test_nested_details_hidden_with_parent() {
        let body = "<details><summary>Outer</summary>\n\n<details><summary>Inner</summary>\n\nhidden\n\n</details>\n\nalso hidden\n\n</details>\n\nafter";
        assert_eq!("▸ Outer\n\nafter", visible_text(body));
    }

    #[test]
    fn test_closed_details_inside_open_details() {
        let body = "<details open><summary>Outer</summary>\n\nshown\n\n<details><summary>Inner</summary>\n\nhidden\n\n</details>\n\n</details>";
        assert_eq!("Outer\n\nshown\n\n▸ Inner", visible_text(body));
    }

    #[test]
    fn test_unclosed_details_hides_rest() {
        let body = "Top\n\n<details><summary>Notes</summary>\n\nhidden";
        assert_eq!("Top\n\n▸ Notes", visible_text(body));
    }

    #[test]
    fn test_stray_close_tag_dropped() {
        assert_eq!("Top\n\nBottom", visible_text("Top\n\n</details>\n\nBottom"));
    }

    #[test]
    fn test_html_comment_block_dropped() {
        let body = "<!-- marker: abc\nmore -->\nVisible";
        assert_eq!("Visible", visible_text(body));
    }

    #[test]
    fn test_inline_html_comment_dropped() {
        assert_eq!(
            "Visible  text",
            visible_text("Visible <!-- hidden --> text")
        );
    }

    #[test]
    fn test_only_comment_is_empty() {
        assert_eq!("", visible_text("<!-- nothing to see -->"));
    }

    #[test]
    fn test_details_in_code_block_kept() {
        let body = "```\n<details><summary>x</summary>\n\ny\n</details>\n```";
        assert_eq!(body, visible_text(body));
    }

    #[test]
    fn test_details_in_code_span_kept() {
        let body = "Use `<details>` and `<!-- -->` here";
        assert_eq!(body, visible_text(body));
    }

    #[test]
    fn test_open_in_attribute_value_stays_closed() {
        let body = "<details title=\"click to open\" data-open=\"1\"><summary>Notes</summary>\n\nhidden\n\n</details>";
        assert_eq!("▸ Notes", visible_text(body));
    }

    #[test]
    fn test_open_attribute_forms() {
        for tag in [
            "<details open=\"\">",
            "<details class='x' OPEN>",
            "<details\nopen>",
        ] {
            let body = format!("{}<summary>Notes</summary>\n\nshown\n\n</details>", tag);
            assert_eq!("Notes\n\nshown", visible_text(&body), "{}", tag);
        }
    }

    #[test]
    fn test_unterminated_comment_hides_rest() {
        assert_eq!(
            "Visible",
            visible_text("Visible\n\n<!-- hidden\n\nmore hidden")
        );
    }

    #[test]
    fn test_crlf_seams_collapse() {
        assert_eq!("A\n\nB", visible_text("A\r\n\r\n<!-- c -->\r\n\r\nB"));
    }

    #[test]
    fn test_unclosed_summary_ends_with_details() {
        let body = "<details><summary>A</details>\n\n<details>\n\nhidden\n\n</details>";
        assert_eq!("▸ A\n\n▸ Details", visible_text(body));
    }

    #[test]
    fn test_other_html_kept() {
        assert_eq!("a <sup>b</sup>", visible_text("a <sup>b</sup>"));
    }
}
