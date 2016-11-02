var handlers = require('../lib/handlers');

describe('handlers', function() {

    var messenger;
    beforeEach(function() {
        messenger= jasmine.createSpyObj('messenger', ['sendToAll']);
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

    });

});
