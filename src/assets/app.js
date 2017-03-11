'use strict';

var app = angular.module('octobot', [ 'ui.router' ]);

app.config(function($stateProvider) {
    $stateProvider.state("login", {
        url: '/login',
        controller: 'LoginController',
        templateUrl : "/login.html"
    })
    .state("users", {
        url: '/users',
        controller: 'UsersController',
        templateUrl : "/users.html"
    })
    .state("repos", {
        url: '/repos',
        controller: 'ReposController',
        templateUrl : "/repos.html"
    });
});

function isLoggedIn() {
  return !!sessionStorage['session'];
}


app.service('sessionHttp', function($http, $state) {
  this.get = function(url) {
    return $http.get(url, {
      headers: {
        session: sessionStorage['session'],
      },
    }).catch(function(e) {
      catch_403(e);
      throw e;
    });
  }

  this.post = function(url, data) {
    return $http.post(url, data, {
      headers: {
        session: sessionStorage['session'],
      },
    }).catch(function(e) {
      catch_403(e);
      throw e;
    });
  };

  function catch_403(e) {
    if (e && e.status == 403) {
      console.log("logging out!");
      delete sessionStorage['session'];
      $state.go("login");
      return true;
    }
    return false;
  }
});

app.run(function($state, $rootScope, sessionHttp) {
  $rootScope.isLoggedIn = isLoggedIn;

  $rootScope.logout = function() {
    sessionHttp.post('/auth/logout', {}).finally(function() {
      delete sessionStorage['session'];
    });
  }

  $rootScope.$on("$stateChangeStart", function(event, toState, toParams, fromState, fromParams) {
    if (!isLoggedIn() && toState.name !== "login")  {
      $state.go("login");
    }
  });

  $rootScope.$on('$stateChangeError', function(event) {
    $state.go('login');
  });

  if (!isLoggedIn() || !$state.current.name) {
    $state.go("login");
  }
});

app.controller('LoginController', function($scope, $state, $http) {

  $scope.username = '';
  $scope.password = '';

  if (isLoggedIn()) {
    $state.go("users");
  }

  $scope.login = function() {
    $http.post('/auth/login', {
      username: $scope.username,
      password: $scope.password,
    }).then(function(resp) {
      console.log("Logged in!");
      sessionStorage['session'] = resp.data.session;
      $state.go("users");

    }).catch(function(e) {
      console.log("Error logging in!" + JSON.stringify(e));
      alert("Access denied");
    });
  };
});


app.controller('UsersController', function($scope, sessionHttp)  {
  $scope.users = [];

  sessionHttp.get('/api/users').then(function(resp) {
    $scope.users = resp.data.users;

    // map it to be more angular friendly
    for (var host in $scope.users) {
      for (var username in $scope.users[host]) {
        $scope.users[host][username]._username = username;
      }
    }
  }).catch(function(e) {
    if (!isLoggedIn()) {
      return;
    }
    alert("Error getting users: " + e);
  });

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
    sessionHttp.post('/api/users', newUsers);
  };
});

app.controller('ReposController', function($scope, sessionHttp)  {

  $scope.repos = [];

  sessionHttp.get('/api/repos').then(function(resp) {
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
    if (!isLoggedIn()) {
      return;
    }

    alert("Error getting repos: " + e);
  });

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
    sessionHttp.post('/api/repos', newRepos);
  };

  $scope.removeRepo = function(host, repo) {
    delete $scope.repos[host][repo];
  }
});
