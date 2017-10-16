'use strict';

var app = angular.module('octobot', [ 'ui.router' ]);

app.config(function($stateProvider) {
    $stateProvider.state('login', {
        url: '/login',
        controller: 'LoginController',
        templateUrl : '/login.html'
    })
    .state('users', {
        url: '/users',
        controller: 'UsersController',
        templateUrl : '/users.html'
    })
    .state('repos', {
        url: '/repos',
        controller: 'ReposController',
        templateUrl : '/repos.html'
    })
    .state('versions', {
        url: '/versions',
        controller: 'VersionsController',
        templateUrl : '/versions.html'
    })
});

function isLoggedIn() {
  return !!sessionStorage['session'];
}

app.run(function($state, $rootScope, sessionHttp) {
  $rootScope.isLoggedIn = isLoggedIn;

  $rootScope.logout = function() {
    sessionHttp.logout();
  };

  $rootScope.$on('$stateChangeStart', function(event, toState, toParams, fromState, fromParams) {
    if (!isLoggedIn() && toState.name !== 'login')  {
      $state.go('login');
    }
  });

  $rootScope.$on('$stateChangeError', function(event) {
    $state.go('login');
  });

  if (!isLoggedIn() || !$state.current.name) {
    $state.go('login');
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
      console.log('logging out!');
      delete sessionStorage['session'];
      $state.go('login');
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

app.service('notificationService', function($rootScope, $timeout) {
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
    $state.go('users');
  }

  $scope.login = function() {
    $http.post('/auth/login', {
      username: $scope.username,
      password: $scope.password,
    }).then(function(resp) {
      notificationService.showSuccess('Logged in successfully');
      sessionStorage['session'] = resp.data.session;
      $state.go('users');

    }).catch(function(e) {
      console.log('Error logging in!' + JSON.stringify(e));
      notificationService.showError('Login failed');
    });
  };
});


function parseError(e) {
  if (e && e.message) {
    return e.message;
  } else if (e && e.status) {
    return 'HTTP ' + e.status;
  } else {
    return e;
  }
}

app.controller('UsersController', function($scope, sessionHttp, notificationService)  {
  $scope.usersMap = {};

  function refresh() {
    return sessionHttp.get('/api/users').then(function(resp) {
      $scope.usersMap = resp.data.users;

    }).catch(function(e) {
      if (!isLoggedIn()) {
        return;
      }
      notificationService.showError('Error getting users: ' + parseError(e));
    });
  }

  $scope.addUser = function(host) {
    $scope.usersMap[host].push({});
  }

  $scope.removeUser = function(host, github_username) {
    for (var i = 0; i < $scope.usersMap[host].length; ++i) {
      if ($scope.usersMap[host][i].github == github_username) {
        $scope.usersMap[host].splice(i, 1);
        return;
      }
    }
  }

  $scope.saveUsers = function() {
    sessionHttp.post('/api/users', $scope.usersMap).then(function() {
      refresh();
      notificationService.showSuccess('Updated users succesfully');
    }).catch(function(e) {
      if (!isLoggedIn()) {
        return;
      }
      notificationService.showError('Error updating users: ' + parseError(e));
    });
  };

  // init
  refresh();
});

app.controller('ReposController', function($rootScope, $scope, sessionHttp, notificationService)  {

  $scope.reposMap = {};

  function refresh() {
    return sessionHttp.get('/api/repos').then(function(resp) {
      $scope.reposMap = resp.data.repos;

      for (var host in $scope.reposMap) {
        for (var repo in $scope.reposMap[host]) {
          var info = $scope.reposMap[host][repo];
          info._repo = repo;

          if (info.force_push_notify == null)  {
            info.force_push_notify = true;
          }
          if (info.jira_versions_enabled == null)  {
            info.jira_versions_enabled = false;
          }
        }
      }
    }).catch(function(e) {
      if (!isLoggedIn()) {
        return;
      }

      notificationService.showError('Error getting repos: ' + parseError(e));
    });
  }

  $scope.addRepo = function(host) {
    $scope.reposMap[host].push({
      force_push_notify: true,
      jira_versions_enabled: false,
    });
  }

  $scope.saveRepos = function() {
    sessionHttp.post('/api/repos', $scope.reposMap).then(function() {
      refresh();
      notificationService.showSuccess('Updated repos succesfully');
    }).catch(function(e) {
      if (!isLoggedIn()) {
        return;
      }
      notificationService.showError('Error updating repos: ' + parseError(e));
    });
  };

  $scope.removeRepo = function(host, repo) {
    for (var i = 0; i < $scope.reposMap[host].length; ++i) {
      if ($scope.reposMap[host][i].repo == repo) {
        $scope.reposMap[host].splice(i, 1);
        return;
      }
    }
  }

  // init
  refresh();
});

app.controller('VersionsController', function($rootScope, $scope, sessionHttp, notificationService)  {

  var jiraBase = null;

  $scope.reset = function() {
    $scope.resp = {};
    $scope.dryRun = true;
    $scope.processing = false;
    $scope.req = {
      project: "",
      version: "",
    };

    $scope.lastVersion = null;
    $scope.lastResp = {};
  }

  function mergeVersions(dryRun) {
    $scope.processing = true;
    let req = {
      project: $scope.req.project,
      version: $scope.req.version,
      dry_run: !!dryRun,
    };
    return sessionHttp.post('/api/merge-versions', req).then(function(resp) {
      $scope.processing = false;
      if (!jiraBase && resp.data.jira_base) {
        jiraBase = resp.data.jira_base;
      }
      return resp;
    }).finally(function(e) {
      $scope.processing = false;
    });
  }

  function mergeVersionsDryRun() {
    $scope.lastResp = {};
    $scope.lastVersion = null;

    mergeVersions(true).then(function(resp) {
      $scope.resp = resp.data.versions;
      $scope.dryRun = false;
    }).catch(function(e) {
      notificationService.showError('Error previewing new version: ' + parseError(e));
    });
  }

  function mergeVersionsForReal() {
    let version = $scope.req.version;

    mergeVersions(false).then(function(resp) {
      notificationService.showSuccess('Created new version succesfully');
      $scope.reset();
      $scope.lastResp = resp.data.versions;
      $scope.lastVersion = version;

    }).catch(function(e) {
      notificationService.showError('Error creating new version: ' + parseError(e));
    });
  }

  $scope.submit = function() {
    if ($scope.dryRun) {
      mergeVersionsDryRun();
    } else{
      mergeVersionsForReal();
    }
  }

  $scope.submitText = function() {
    if ($scope.dryRun) {
      return "Preview";
    } else {
      return "Submit";
    }
  };

  $scope.hasRespData = function() {
    return Object.keys($scope.resp).length > 0;
  }

  $scope.jiraLink = function(key) {
    if (!jiraBase) {
      return "";
    } else {
      return jiraBase + "/browse/" + key;
    }
  }

  $scope.reset();
});
