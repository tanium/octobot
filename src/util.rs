fn escape_for_slack<S: Into<String>>(str: S) -> String {
    str.into().replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
}

pub fn make_link<S: Into<String>>(url: S, text: S) -> String {
    format!("<{}|{}>", escape_for_slack(url.into()), escape_for_slack(text.into()))
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
