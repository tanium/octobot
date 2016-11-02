var util = require('../lib/util');

describe("util", function() {

    describe("makeLink", function() {

        it("should make a slack link", function() {
            expect(util.makeLink("http://the-url", "the text")).toEqual('<http://the-url|the text>');
        });

        it("should HTML escape &, < and >", function() {
            expect(util.makeLink("http://the-url&hello=<>", "the text & <> stuff"))
                .toEqual('<http://the-url&amp;hello=&lt;&gt;|the text &amp; &lt;&gt; stuff>');
        });

    });
});
