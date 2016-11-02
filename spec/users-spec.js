var users = require('../lib/users');

describe("users", function() {

    afterEach(function() {
        users.setUsersMapForTesting({});
    });

    describe("slackUserName", function() {
        it("should default to github name", function() {
            expect(users.slackUserName("joe")).toEqual("joe");
        });
        it("should replace dashes with dots per our slack convention", function() {
            expect(users.slackUserName("joe-smith")).toEqual("joe.smith");
        });
        it("should return mapped names if defined for repo", function() {
            users.setUsersMapForTesting({ 'git.company.com': { 'joe-smith': 'the-real-joe-smith' } });
            expect(users.slackUserName("joe-smith", { html_url: 'http://git.company.com' })).toEqual("the-real-joe-smith");
        });
    });

    describe("mention", function() {
        it("should @mention the user", function() {
            expect(users.mention("me")).toEqual("@me");
        });
    });

    describe("slackUserRef", function() {
        it("should @mention the slack user", function() {
            expect(users.slackUserRef("joe-smith")).toEqual("@joe.smith");
        });
        it("should @mention the mapped user", function() {
            users.setUsersMapForTesting({ 'git.company.com': { 'joe-smith': 'the-real-joe-smith' } });
            expect(users.slackUserRef("joe-smith", { html_url: 'http://git.company.com' })).toEqual("@the-real-joe-smith");
       });
    });

    describe("desiresPeaceAndQuiet", function() {
        it("should be true if user is configured for peace and quiet", function() {
            users.setUsersMapForTesting({ 'git.company.com': { 'joe-smith': 'DO NOT DISTURB' } });
            expect(users.desiresPeaceAndQuiet("joe-smith", { html_url: 'http://git.company.com' })).toBe(true);
            expect(users.desiresPeaceAndQuiet("bob", { html_url: 'http://git.company.com' })).toBe(false);
            expect(users.desiresPeaceAndQuiet('DO NOT DISTURB')).toBe(true);
        });
    });
});
