var handlers = require('../lib/handlers');

describe('handlers', function() {

    var messenger;
    beforeEach(function() {
        messenger= jasmine.createSpyObj('messenger', ['sendToAll']);
    });


    describe("pullRequestCommentHandler ", function() {
        var data;
        beforeEach(function() {
            data = {
                action: 'created',
                comment: {
                    user: {
                        login: 'the-commenter',
                    },
                    body: 'i have something to say',
                    html_url: 'http://the-PR-comment',
                },
                pull_request: {
                    html_url: 'http://the-pr',
                    title: 'MyPR',
                    user: {
                        login: 'the-owner'
                    },
                }
            };
        });

        it("should send comments", function() {
            handlers.pullRequestCommentHandler (messenger)(data);

            expect(messenger.sendToAll).toHaveBeenCalledWith(
                'Comment on "<http://the-pr|MyPR>"',
                [{
                    title: 'the.commenter said:',
                    title_link: 'http://the-PR-comment',
                    text: 'i have something to say',
                }],
                data.pull_request,
                data.repository,
                data.sender
            );
        });

        it("should not send empty comments", function() {
            data.comment.body = '  ';
            handlers.pullRequestCommentHandler(messenger)(data);

            data.comment.body = null;
            handlers.pullRequestCommentHandler(messenger)(data);

            expect(messenger.sendToAll).not.toHaveBeenCalled();
        });

    });

    describe("pullRequestReviewHandler", function() {
        var data;
        beforeEach(function() {
            data = {
                action: 'submitted',
                review: {
                    state: null,
                    user: {
                        login: 'the-reviewer',
                    },
                    body: 'i also commented',
                    html_url: 'http://the-review',
                },
                pull_request: {
                    html_url: 'http://the-pr',
                    title: 'MyPR',
                    user: {
                        login: 'the-owner'
                    },
                }
            };
        });

        it("should send approved reviews", function() {
            data.review.state = 'approved';
            handlers.pullRequestReviewHandler(messenger)(data);

            expect(messenger.sendToAll).toHaveBeenCalledWith(
                'the.reviewer approved PR "<http://the-pr|MyPR>"',
                [{
                    title: 'Review: Approved',
                    title_link: 'http://the-review',
                    text: 'i also commented',
                    color: 'good',
                }],
                data.pull_request,
                data.repository,
                data.sender
            );
        });

        it("should send rejected reviews", function() {
            data.review.state = 'changes_requested';
            handlers.pullRequestReviewHandler(messenger)(data);

            expect(messenger.sendToAll).toHaveBeenCalledWith(
                'the.reviewer requested changes to PR "<http://the-pr|MyPR>"',
                [{
                    title: 'Review: Changes Requested',
                    title_link: 'http://the-review',
                    text: 'i also commented',
                    color: 'danger',
                }],
                data.pull_request,
                data.repository,
                data.sender
            );
        });

        it("should send review comments", function() {
            data.review.state = 'commented';
            handlers.pullRequestReviewHandler(messenger)(data);

            expect(messenger.sendToAll).toHaveBeenCalledWith(
                'Comment on "<http://the-pr|MyPR>"',
                [{
                    title: 'the.reviewer said:',
                    title_link: 'http://the-review',
                    text: 'i also commented',
                }],
                data.pull_request,
                data.repository,
                data.sender
            );
        });

        it("should not send empty review comments", function() {
            data.review.state = 'commented';

            data.review.body = '  ';
            handlers.pullRequestReviewHandler(messenger)(data);

            data.review.body = null;
            handlers.pullRequestReviewHandler(messenger)(data);

            expect(messenger.sendToAll).not.toHaveBeenCalled();
        });
    });
});
