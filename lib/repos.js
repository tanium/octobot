var fs = require('fs');
var url = require('url');

// map: { github-host -> { repo-full-name -> slack-channel-name } }
// can specify repo full name or just org name
var g_repos = {};

// just load on startup
if (fs.existsSync('repos.json')) {
    g_repos = JSON.parse(fs.readFileSync('repos.json'));
}

function lookupRepoChannel(repo) {
    if (repo) {
        var host = url.parse(repo.html_url).host;
        var reposForHost = g_repos[host];
        if (reposForHost) {
            var channelForRepo = reposForHost[repo.full_name];
            if (channelForRepo) {
                return channelForRepo;
            }
            var channelForOrg = reposForHost[repo.owner.login];
            if (channelForOrg) {
                return channelForOrg;
            }
        }
    }

    return null;
}

exports.setReposMapForTesting = function(reposMap) {
    g_repos = reposMap;
}

exports.getChannel = function(repo) {
    return lookupRepoChannel(repo);
};
