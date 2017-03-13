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

app.run(function($state, $rootScope, sessionHttp) {
  $rootScope.isLoggedIn = isLoggedIn;

  $rootScope.logout = function() {
    sessionHttp.logout();
  };

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

app.service('sessionHttp', function($http, $state) {
  var self = this;
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

  this.logout = function() {
    self.post('/auth/logout', null).finally(function() {
      console.log("logging out!");
      delete sessionStorage['session'];
      $state.go("login");
    });
  }

  function catch_403(e) {
    if (e && e.status == 403) {
      self.logout();
      return true;
    }
    return false;
  }
});

app.service("notificationService", function($rootScope, $timeout) {
  var self = this;

  this.showError = function(msg) {
    $rootScope.errorMessage = msg;
    $timeout(function() {
      $rootScope.errorMessage = null;
    }, 5000);
  };

  this.showSuccess = function(msg) {
    $rootScope.successMessage = msg;
    $timeout(function() {
      $rootScope.successMessage = null;
    }, 3000);
  };
});

app.controller('LoginController', function($scope, $state, $http, notificationService) {

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
      notificationService.showSuccess("Logged in successfully");
      sessionStorage['session'] = resp.data.session;
      $state.go("users");

    }).catch(function(e) {
      console.log("Error logging in!" + JSON.stringify(e));
      notificationService.showError("Login failed");
    });
  };
});


function parseError(e) {
  if (e && e.message) {
    return e.message;
  } else if (e && e.status) {
    return "HTTP " + e.status;
  } else {
    return e;
  }
}

app.controller('UsersController', function($scope, sessionHttp, notificationService)  {
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
    notificationService.showError("Error getting users: " + parseError(e));
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
    sessionHttp.post('/api/users', newUsers).then(function() {
      notificationService.showSuccess("Updated users succesfully");
    }).catch(function(e) {
      if (!isLoggedIn()) {
        return;
      }
      notificationService.showError("Error updating users: " + parseError(e));
    });
  };
});

app.controller('ReposController', function($rootScope, $scope, sessionHttp, notificationService)  {

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
      }
    }
  }).catch(function(e) {
    if (!isLoggedIn()) {
      return;
    }

    notificationService.showError("Error getting repos: " + parseError(e));
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
          info.force_push_reapply_statuses = info.force_push_reapply_statuses.split(/\s*,\s*/);
        }
      }
    }
    sessionHttp.post('/api/repos', newRepos).then(function() {
      notificationService.showSuccess("Updated repos succesfully");
    }).catch(function(e) {
      if (!isLoggedIn()) {
        return;
      }
      notificationService.showError("Error updating repos: " + parseError(e));
    });
  };

  $scope.removeRepo = function(host, repo) {
    delete $scope.repos[host][repo];
  }
});
