var reqjs = require('requirejs');
var fs = require('fs');
var path = require('path');
var Q = require('q');

var github = reqjs('approve.js/lib/github');
var auth = reqjs('approve.js/lib/node/auth');

var g_askPassScript = path.join(__dirname, '../git-ask-pass.js');
var g_availCloneDirs = {};
var g_cloneDirNum = 1;
var g_sessions = {};

function mkdirp(dir) {
    if (fs.existsSync(dir)) {
        return;
    }
    var parent = path.dirname(dir);
    if (!fs.existsSync(parent)) {
        mkdirp(parent);
    }
    fs.mkdirSync(dir);
}

function getGitUser(host) {
    var tokenData = auth.getTokenData(host);
    return tokenData.username;
}

function getGitToken(host) {
    var tokenData = auth.getTokenData(host);
    return tokenData.token;
}

function getOrInitSession(host) {
    if (g_sessions[host]) {
        return Q.when(g_sessions[host]);
    }
    var token = getGitToken(host);
    var sess = github.newSession(host);
    return sess.authenticateWithToken(token).then(function() {

        sess.errorHandler = function(result) {
            // skip the logging.
            return Q.reject(result);
        };

        g_sessions[host] = sess;
        return sess;
    });
}

function cloneDirKey(host, owner, repo) {
    return host + ':' + owner + '/' + repo;
}

function initCloneDir(host, owner, repo) {
    var key = cloneDirKey(host, owner, repo);
    g_availCloneDirs[key] = [
        makeCloneDir(host, owner, repo, String(g_cloneDirNum++)),
        makeCloneDir(host, owner, repo, String(g_cloneDirNum++)),
        makeCloneDir(host, owner, repo, String(g_cloneDirNum++)),
        makeCloneDir(host, owner, repo, String(g_cloneDirNum++)),
        makeCloneDir(host, owner, repo, String(g_cloneDirNum++)),
    ];
}

function makeCloneDir(host, owner, repo, num) {
    return path.join('.', 'repos', host, owner, repo, num);
}

function returnCloneDir(host, owner, repo, dir) {
    if (dir) {
        var key = cloneDirKey(host, owner, repo);
        g_availCloneDirs[key].push(dir);
    }
}

function takeCloneDir(host, owner, repo, timeSlept) {
    var key = cloneDirKey(host, owner, repo);
    if (!g_availCloneDirs[key]) {
        initCloneDir(host, owner, repo);
    }

    var nextAvail = g_availCloneDirs[key].pop();
    if (nextAvail) {
        return Q.when(nextAvail);
    }

    // if > 1min, then just start over with clone dirs!
    if (timeSlept && timeSlept > 60 * 1000) {
        initCloneDir(host, owner, repo);
        return takeCloneDir(host, owner, repo, callback, 0);
    }

    timeSlept = timeSlept || 0;
    var timeToSleep = 500;

    var defer = Q.defer();

    setTimeout(function() {
        defer.resolve(takeCloneDir(host, owner, repo, callback, timeSlept + timeToSleep));
    }, timeToSleep);

    return defer.promise;
}

// always run git safely: no exiting and always with auth.
function runGitSafely(args, dir, stdin) {
    var git = reqjs('approve.js/lib/node/git');
    if (!dir) {
        Q.reject("No working directory provided! Args=" + args);
    }
    return git.runGit(args, { exitOnFailure: false, cwd: dir, env: { 'GIT_ASKPASS': g_askPassScript }, stdin: stdin } );
}

function cloneRepo(host, owner, repo) {
    var gitUser = getGitUser(host);
    if (!gitUser) {
        Q.reject("No git user configured for " + host);
    }

    return takeCloneDir(host, owner, repo).then(function(cloneDir) {
        var url = 'https://' + gitUser + '@' + host + "/" + owner + "/" + repo;

        var clonePromise;
        if (fs.existsSync(path.join(cloneDir, ".git"))) {
            clonePromise = runGitSafely(['fetch'], cloneDir);
        } else {
            mkdirp(cloneDir);
            clonePromise = runGitSafely(['clone', url, '.'], cloneDir);
        }

        return clonePromise.then(function() {
            return cloneDir;
        }).catch(function(e) {
            returnCloneDir(host, owner, repo, cloneDir);
            return Q.reject("Failed to clone repo: " + e);
        });
    });
}

function getCommitDescription(cloneDir, commitHash) {
    return runGitSafely(['log', '-1', '--pretty=%B', commitHash], cloneDir).then(function(output) {
        var lines = output.split("\n");
        lines = lines.map(function(l) { return l.trim(); });

        if (lines.length === 0) {
            return Q.reject("Empty commit message found!");
        }

        var title = lines[0];

        var body = "";
        // skip the blank line
        if (lines.length > 2) {
            body = lines.slice(2).join("\n");
            body += "\n";
        }

        return [title, body];
    });
}

function cherryPick(cloneDir, commitHash, prBranchName, prNumber, targetBranch, origBaseBranch) {
    var realTargetBranch = 'origin/' + targetBranch;

    var title, body;

    return runGitSafely(['reset', '--hard', realTargetBranch], cloneDir).then(function() {
        return runGitSafely(['clean', '-fdx'], cloneDir);
    }).then(function() {
        return runGitSafely(['rev-parse', '--abbrev-ref', 'HEAD'], cloneDir);
    }).then(function(currentBranch) {
        if (currentBranch == prBranchName) {
            return Q.when();
        } else {
            return runGitSafely(['branch', '-d', prBranchName], cloneDir).catch(function() {
                // ignore errors if branch doesn't exist yet
            }).then(function() {
                return runGitSafely(['checkout', '-f', '-b', prBranchName, realTargetBranch], cloneDir);
            });
        }
    }).then(function() {
        return runGitSafely(['cherry-pick', '-X', 'ignore-all-space', commitHash], cloneDir);
    }).then(function() {
        return getCommitDescription(cloneDir, commitHash).then(function(desc) {
            // grab original title and strip out the PR number at the end
            var origTitle = String(desc[0]).replace(/(\s*\(#\d+\))+$/, '');
            // strip out 'release' from the prefix to keep titles shorter
            var prefix = origBaseBranch + "->" + targetBranch.replace(/^release\//, '') ;

            title = prefix + ": " + origTitle;

            body = (desc[1] || "").trim();
            if (body.length != 0) {
                body += "\n\n";
            }
            body += "(cherry-picked from " + commitHash.substr(0, 7) + ", PR #" + prNumber + ")";

            return runGitSafely(['commit', '--amend', '-F', '-'], cloneDir, title + "\n\n" + body);
        });
    }).then(function() {
        return [title, body];
    });
}

function pushToOrigin(cloneDir, prBranchName) {
    return runGitSafely(['ls-remote', '--heads'], cloneDir).then(function(heads) {
        if (heads.indexOf('refs/heads/' + prBranchName) >= 0) {
            return Q.reject("PR branch already exists on origin: " + prBranchName);
        } else {
            return runGitSafely(['push', 'origin', prBranchName + ":" + prBranchName], cloneDir);
        }
    });
}

exports.createMergePR = function(host, owner, repo, pullRequestNumber, targetBranch) {
    var toReturn;
    var session;

    return getOrInitSession(host).then(function(sess) {
        session = sess;

        return github.getPullRequest({
            owner: owner,
            repo: repo,
            number: String(pullRequestNumber),
        }, session);

    }).then(function(pullRequest) {
        if (!pullRequest.data.merged) {
            return Q.reject("Pull Request #" + pullRequestNumber + " is not yet merged.");
        }
        if (!pullRequest.data.merge_commit_sha) {
            return Q.reject("Pull Request #" + pullRequestNumber + " has no merge commit.");
        }

        var prBranchName = pullRequest.data.head.ref.replace(/.*\//, '') + "-" + targetBranch.replace(/.*\//, '');
        var commitHash = pullRequest.data.merge_commit_sha;
        var origBaseBranch = pullRequest.data.base.ref;

        return github.getAllPullRequests({
            owner: owner,
            repo: repo,
        }, session).then(function(pullRequests) {
            var samePR = pullRequests.data.find(function(pr)  {
                return pr.state == 'open' && pr.head.ref == prBranchName;
            });

            if (samePR) {
                return Q.reject("Pull request already opened for branch '" + prBranchName + "': " + samePR.html_url);
            }
            return Q.when();

        }).then(function() {
            return cloneRepo(host, owner, repo)

        }).then(function(cloneDir) {
            toReturn = cloneDir;

            return cherryPick(cloneDir, commitHash, prBranchName, pullRequestNumber, targetBranch, origBaseBranch);

        }).then(function(desc) {
            var title = desc[0];
            var body = desc[1] || "";

            return pushToOrigin(toReturn, prBranchName).then(function() {
                return github.createPullRequest({
                    owner: owner,
                    repo: repo,
                    title: title,
                    body: body,
                    head: prBranchName,
                    base: targetBranch,
               }, session).then(function(newPRResult) {
                  // assign PR w/ original assignees, but still return new PR result
                  var assignees = pullRequest.data.assignees.map(function(a) { return a.login; });
                  return github.assignPullRequest({
                      owner: owner,
                      repo: repo,
                      number: newPRResult.data.number,
                      assignees: assignees,
                  }, session).then(function() {
                      return newPRResult;
                  });
               });
            });
        });
    }).finally(function() {
        returnCloneDir(host, owner, repo, toReturn);
    });
}

exports.getPullRequestLabels = function(host, owner, repo, pullRequestNumber) {
    return getOrInitSession(host).then(function(sess) {
        session = sess;

        return github.getPullRequestLabels({
            owner: owner,
            repo: repo,
            number: String(pullRequestNumber),
        }, session);

    });
}
