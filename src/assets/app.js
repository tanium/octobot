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

  this.put = function(url, data) {
    return $http.put(url, data, {
      headers: {
        session: sessionStorage['session'],
      },
    }).catch(function(e) {
      catch_403(e);
      throw e;
    });
  };

  this.delete = function(url) {
    return $http.delete(url, {
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

  function init() {
    $('#add-user-modal').on('shown.bs.modal', function () {
      $('#add-user-username').focus()
    });

    refresh();
  }

  function refresh() {
    return sessionHttp.get('/api/users').then(function(resp) {
      $scope.users = resp.data.users;

    }).catch(function(e) {
      if (!isLoggedIn()) {
        return;
      }
      notificationService.showError('Error getting users: ' + parseError(e));
    });
  }

  $scope.addUser = function() {
    $scope.newUser = {};
    $('#add-user-modal').modal('show');
  }

  $scope.addUserSubmit = function() {
    $('#add-user-modal').modal('hide');

    sessionHttp.post('/api/users', $scope.newUser).then(function(resp) {
      notificationService.showSuccess('Added user succesfully');
      refresh()
    }).catch(function(e) {
      if (!isLoggedIn()) {
        return;
      }
      notificationService.showError('Error removing user: ' + parseError(e));
    });
  }

  $scope.removeUser = function(user) {
    if (!confirm("Are you sure you want to delete user " + user.github + "?")) {
      return;
    }
    return sessionHttp.delete('/api/user?id=' + Number(user.id)).then(function(resp) {
      notificationService.showSuccess('Remove user succesfully');
      refresh();
    }).catch(function(e) {
      if (!isLoggedIn()) {
        return;
      }
      notificationService.showError('Error removing user: ' + parseError(e));
    });
  }

  // init
  init();
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

  function init() {
    $scope.reset();
    $('#auth-modal').on('shown.bs.modal', function () {
      $('#auth-username').focus()
    });
    // clear the password before show and after hide
    $('#auth-modal').on('show.bs.modal', function() {
        $scope.req.admin_pass = "";
    });
    $('#auth-modal').on('hidden.bs.modal', function() {
        $scope.req.admin_pass = "";
    });
  }

  $scope.reset = function() {
    $scope.resp = {};
    $scope.dryRun = true;
    $scope.processing = false;
    $scope.req = {
      project: "",
      version: "",
      admin_pass: "",
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
      admin_user: $scope.req.admin_user,
      admin_pass: $scope.req.admin_pass,
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
    } else {
      $('#auth-modal').modal('show');
    }
  }

  $scope.modalSubmit = function() {
      mergeVersionsForReal();
      $('#auth-modal').modal('hide');
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

  init();
});
