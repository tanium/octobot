'use strict';

var app = angular.module('octobot', []);

app.controller('OctobotController', function($scope) {

  $scope.isLoggedIn = function() {
      return !!sessionStorage['session'];
  };

});

app.controller('LoginController', function($scope, $http) {

  $scope.username = 'admin';
  $scope.password = '';

  $scope.login = function() {
    $http.post('/auth/login', {
      username: $scope.username,
      password: $scope.password,
    }).then(function(resp) {
      console.log("Logged in!");
      sessionStorage['session'] = resp.data.session;

    }).catch(function(e) {
      console.log("Error logging in!" + JSON.stringify(e));
    });
  };
});


app.controller('AdminController', function($scope, $http) {

  function http_get(url) {
    return $http.get(url, {
      headers: {
        session: sessionStorage['session'],
      },
    })
  }

  function http_post(url, data) {
    return $http.post(url, data, {
      headers: {
        session: sessionStorage['session'],
      },
    })
  }

  $scope.state = 'users';
  $scope.users = [];
  $scope.repos = [];

  http_get('/api/users').then(function(resp) {
    $scope.users = resp.data.users;

    // map it to be more angular friendly
    for (var host in $scope.users) {
      for (var username in $scope.users[host]) {
        $scope.users[host][username]._username = username;
      }
    }
  }).catch(function(e) {
    alert("Error getting users: " + e);
  });

  http_get('/api/repos').then(function(resp) {
    $scope.repos = resp.data.repos;

    for (var host in $scope.repos) {
      for (var repo in $scope.repos[host]) {
        var info = $scope.repos[host][repo];
        info._repo = repo;

        if (info.force_push_notify == null)  {
          info.force_push_notify = true;
        }
        if (info.jira_enabled == null)  {
          info.jira_enabled = true;
        }
        if (info.force_push_reapply_statuses != null) {
          info.force_push_reapply_statuses = info.force_push_reapply_statuses.join(",");
        }
        if (info.version_script != null) {
          info.version_script = info.version_script.join(",");
        }
      }
    }
  }).catch(function(e) {
    alert("Error getting repos: " + e);
  });

  $scope.logout = function() {
    http_post('/auth/logout', {}).finally(function() {
      delete sessionStorage['session'];
    });
  }

  $scope.addUser = function(host) {
    $scope.users[host]["new-user-" + Math.random()] = {};
  }

  $scope.removeUser = function(host, username) {
    delete $scope.users[host][username];
  }

  $scope.saveUsers = function() {
    // remap to make sure edited usersnames correspodn to keys
    var newUsers = {};
    for (var host in $scope.users) {
      newUsers[host] = {};
      for (var key in $scope.users[host]) {
        var info = $scope.users[host][key];
        newUsers[host][info._username] = info;
      }
    }
    http_post('/api/users', newUsers);
  };

  $scope.addRepo = function(host) {
    $scope.repos[host]["new-repo-" + Math.random()] = {
      force_push_notify: true,
      jira_enabled: true,
    };
  }

  $scope.saveRepos = function() {
    // remap to make sure edited usersnames correspodn to keys
    var newRepos = {};
    for (var host in $scope.repos) {
      newRepos[host] = {};
      for (var key in $scope.repos[host]) {
        var info = $scope.repos[host][key];
        newRepos[host][info._repo] = info;
        if (info.force_push_reapply_statuses) {
          info.force_push_reapply_statuses = info.force_push_reapply_statuses.split(/\s*[,;]\s*/);
        }
        if (info.version_script) {
          info.version_script = info.version_script.split(/\s*[,;]\s*/);
        }
      }
    }
    http_post('/api/repos', newRepos);
  };

  $scope.removeRepo = function(host, repo) {
    delete $scope.repos[host][repo];
  }
});
