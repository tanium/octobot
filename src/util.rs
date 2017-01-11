fn escape_for_slack(str: &str) -> String {
    str.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
}

pub fn make_link(url: &str, text: &str) -> String {
    "<".to_string() + escape_for_slack(url).as_str() + "|" + escape_for_slack(text).as_str() + ">"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_make_link() {
        assert_eq!("<http://the-url|the text>",
                   make_link("http://the-url", "the text"));
    }

    #[test]
    fn test_make_link_escapes() {
        assert_eq!("<http://the-url&amp;hello=&lt;&gt;|the text &amp; &lt;&gt; stuff>",
                   make_link("http://the-url&hello=<>", "the text & <> stuff"));
    }
}
