var handlers = require('../lib/handlers');
var Q = require('q');

describe('handlers', function() {

    var messenger, githubAPI;
    beforeEach(function() {
        messenger = jasmine.createSpyObj('messenger', ['sendToAll', 'sendToOwner']);
        githubAPI = jasmine.createSpyObj('githubAPI', ['createMergePR', 'getPullRequestLabels']);
    });

    describe("pullRequestHandler ", function() {
        var data;
        beforeEach(function() {
            data = {
                action: '',
                pull_request: {
                    html_url: 'http://the-pr',
                    title: 'MyPR',
                    user: {
                        login: 'the-owner'
                    },
                    number: 22,
                },
                repository: {
                    html_url: 'http://git.com/the-owner/the-repo',
                    owner: {
                        login: 'the-owner',
                    },
                    name: 'the-repo',
                }
            };
        });

        it("should send messages on on open", function() {
            data.action = 'opened';
            handlers.pullRequestHandler(messenger, githubAPI)(data);

            expect(messenger.sendToAll).toHaveBeenCalledWith(
                'Pull Request opened by the.owner',
                [{
                  title: 'Pull Request #22: "MyPR"',
                    title_link: 'http://the-pr',
                }],
                data.pull_request,
                data.repository,
                data.sender
            );
        });

        it("should send messages on on close", function() {
            data.action = 'closed';
            data.pull_request.merged = false;
            handlers.pullRequestHandler(messenger, githubAPI)(data);

            expect(messenger.sendToAll).toHaveBeenCalledWith(
                'Pull Request closed',
                [{
                  title: 'Pull Request #22: "MyPR"',
                    title_link: 'http://the-pr',
                }],
                data.pull_request,
                data.repository,
                data.sender
            );
        });

        it("should send messages on on merge", function() {
            data.action = 'closed';
            data.pull_request.merged = true;
            githubAPI.getPullRequestLabels.and.returnValue(Q.when());

            handlers.pullRequestHandler(messenger, githubAPI)(data);

            expect(messenger.sendToAll).toHaveBeenCalledWith(
                'Pull Request merged',
                [{
                  title: 'Pull Request #22: "MyPR"',
                    title_link: 'http://the-pr',
                }],
                data.pull_request,
                data.repository,
                data.sender
            );
        });

        it("should open merge PRs on merge", function(done) {
            data.action = 'closed';
            data.pull_request.merged = true;

            var labelsPromise = Q.when({
                data: [
                    { name: 'backport-1.0' },
                    { name: 'backport-2.0' },
                    { name: 'some-other' },
                ],
            });
            var mergePromise = Q.when();
            githubAPI.getPullRequestLabels.and.returnValue(labelsPromise);
            githubAPI.createMergePR.and.returnValue(mergePromise);

            handlers.pullRequestHandler(messenger, githubAPI)(data);

            expect(messenger.sendToAll).toHaveBeenCalledWith(
                'Pull Request merged',
                [{
                    title: 'Pull Request #22: "MyPR"',
                    title_link: 'http://the-pr',
                }],
                data.pull_request,
                data.repository,
                data.sender
            );

            Q.all([labelsPromise, mergePromise]).then(function() {
                expect(githubAPI.createMergePR.calls.allArgs()).toEqual([
                    [ "git.com", "the-owner", "the-repo", 22, "release/1.0" ],
                    [ "git.com", "the-owner", "the-repo", 22, "release/2.0" ],
                ]);
            }).finally(function() {
                done();
            });
        });


        it("should send messages on on assign", function() {
            data.action = 'assigned';
            data.pull_request.assignees = [
                { login: 'joe' },
                { login: 'bob-smith' },
            ];

            handlers.pullRequestHandler(messenger, githubAPI)(data);

            expect(messenger.sendToAll).toHaveBeenCalledWith(
                'Pull Request assigned to @joe, @bob.smith',
                [{
                    title: 'Pull Request #22: "MyPR"',
                    title_link: 'http://the-pr',
                }],
                data.pull_request,
                data.repository,
                data.sender
            );
        });

        it("should send messages on on unassign", function() {
            data.action = 'unassigned';
            handlers.pullRequestHandler(messenger, githubAPI)(data);

            expect(messenger.sendToAll).toHaveBeenCalledWith(
                'Pull Request unassigned',
                [{
                  title: 'Pull Request #22: "MyPR"',
                    title_link: 'http://the-pr',
                }],
                data.pull_request,
                data.repository,
                data.sender
            );
        });

        it("should create a merge PR on appropriate label", function(done) {
            data.action = 'labeled';
            data.pull_request.merged = true;
            data.label = {
                name: 'backport-1.5',
            };

            var successPromise = Q.when();

            githubAPI.createMergePR.and.returnValue(successPromise);
            handlers.pullRequestHandler(messenger, githubAPI)(data);

            expect(githubAPI.createMergePR).toHaveBeenCalledWith('git.com', 'the-owner', 'the-repo', 22, 'release/1.5');

            successPromise.then(function() {
                expect(messenger.sendToOwner).toHaveBeenCalledWith(
                    'Created merge Pull Request',
                    [{
                        title: 'Source PR: #22: "MyPR"',
                        title_link: 'http://the-pr'
                    }],
                    data.pull_request,
                    data.repository
                );
             }).finally(done);
        });

        it("should send messages on failed merge PR creation", function(done) {
            data.action = 'labeled';
            data.pull_request.merged = true;
            data.label = {
                name: 'backport-1.5',
            };

            var failPromise = Q.reject("I just can't do it!");

            githubAPI.createMergePR.and.returnValue(failPromise);
            handlers.pullRequestHandler(messenger, githubAPI)(data);

            expect(githubAPI.createMergePR).toHaveBeenCalledWith('git.com', 'the-owner', 'the-repo', 22, 'release/1.5');

            failPromise.finally(function() {
                expect(messenger.sendToOwner).toHaveBeenCalledWith(
                    'Error creating merge Pull Request',
                    [{
                        title: 'Source PR: #22: "MyPR"',
                        title_link: 'http://the-pr',
                        color: 'danger',
                        text: "I just can't do it!"
                    }],
                    data.pull_request,
                    data.repository
                );
             }).finally(done);
        });

        it("should not create a merge PR for non-merged PRs", function() {
            data.action = 'labeled';
            data.pull_request.merged = false;
            data.label = {
                name: 'backport-1.5',
            };
            handlers.pullRequestHandler(messenger, githubAPI)(data);
            expect(githubAPI.createMergePR).not.toHaveBeenCalled();
        });

        it("should not create a merge PR for non-backport labels", function() {
            data.action = 'labeled';
            data.pull_request.merged = false;
            data.label = {
                name: 'some-other-1.5',
            };
            handlers.pullRequestHandler(messenger, githubAPI)(data);
            expect(githubAPI.createMergePR).not.toHaveBeenCalled();
        });
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
            handlers.pullRequestCommentHandler(messenger, githubAPI)(data);

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
            handlers.pullRequestCommentHandler(messenger, githubAPI)(data);

            data.comment.body = null;
            handlers.pullRequestCommentHandler(messenger, githubAPI)(data);

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
            handlers.pullRequestReviewHandler(messenger, githubAPI)(data);

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
            handlers.pullRequestReviewHandler(messenger, githubAPI)(data);

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
            handlers.pullRequestReviewHandler(messenger, githubAPI)(data);

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
            handlers.pullRequestReviewHandler(messenger, githubAPI)(data);

            data.review.body = null;
            handlers.pullRequestReviewHandler(messenger, githubAPI)(data);

            expect(messenger.sendToAll).not.toHaveBeenCalled();
        });
    });
});
