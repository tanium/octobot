
function escapeForSlack(str) {
    return str.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;");
}

exports.makeLink = function(url, text) {
    return "<" + escapeForSlack(url) + "|" + escapeForSlack(text) + ">";
};
