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

// Note: this is stored client side for convenience only, not used for any auth purposes.
function loggedInUser() {
  return sessionStorage['username'];
}

app.run(function($state, $rootScope, $timeout, sessionHttp, notificationService) {
  $rootScope.isLoggedIn = isLoggedIn;

  $rootScope.logout = function() {
    sessionHttp.logout();
  };

  $rootScope.$on('$stateChangeStart', function(event, toState, toParams, fromState, fromParams) {
    if (!isLoggedIn() && toState.name !== 'login')  {
      event.preventDefault();
      $state.go('login');
    }
  });

  var checkPromise = null;
  const checkInterval = 30 * 1000;

  function checkSession() {
    if (isLoggedIn()) {
      sessionHttp.post('/auth/check', {}).then(function() {
        checkPromise = $timeout(checkSession, checkInterval);
      }).catch(function(e) {
        if (e && e.status == 403) {
          notificationService.showError("Session expired: Logged out");
        } else {
          checkPromise = $timeout(checkSession, checkInterval);
        }
      });
    } else {
      if ($state.current.name != 'login') {
        console.log("Redirecting to login page");
        $state.go('login');
      }

      $rootScope.$on('octobot.login', function() {
        console.log("Logged in. Starting check-session loop");
        if (checkPromise) {
          $timeout.cancel(checkPromise);
        }
        checkSession();
      });
    }
  }

  $rootScope.$on('$stateChangeError', function(event) {
    $state.go('login');
  });

  checkSession();
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

  function postLogout() {
    console.log('Logging out!');
    sessionStorage.clear();
    $state.go('login');
  }

  this.logout = function() {
    if (!isLoggedIn()) {
      postLogout();
    } else {
      self.post('/auth/logout', null).finally(function() {
        postLogout();
      });
    }
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

app.controller('LoginController', function($scope, $rootScope, $state, $http, notificationService) {

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
      sessionStorage['username'] = $scope.username;
      $rootScope.$emit('octobot.login');
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
    $scope.errorMessage = "";

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

  $scope.editUser = function(user) {
    $scope.theUser = user;
    $('#add-user-modal').modal('show');
  }

  $scope.addUser = function() {
    $scope.theUser = {
      mute_direct_messages: false,
      mute_team_direct_messages: false,
      muted_repos: [],
    };
    $('#add-user-modal').modal('show');
  }

  $scope.addUserSubmit = function() {
    var editing = !!$scope.theUser.id;
    $scope.errorMessage = "";

    sessionHttp.post('/api/users/verify?email=' + $scope.theUser.email).then(function(resp) {
      if (!resp.data.id || !resp.data.name) {
        $scope.errorMessage = 'Failed to lookup slack username for given email address';
        return;
      }

      $('#add-user-modal').modal('hide');
      $scope.theUser.slack_name = resp.data.name;
      $scope.theUser.slack_id = resp.data.id;

      if (editing) {
        doEditUser();
      } else {
        doAddUser();
      }

    }).catch(function(e) {
      $scope.errorMessage = 'Error verifying email address: ' + parseError(e);
    });

  }

  function doAddUser() {
    sessionHttp.post('/api/users', $scope.theUser).then(function(resp) {
      notificationService.showSuccess('Added user succesfully');
      refresh()
    }).catch(function(e) {
      if (!isLoggedIn()) {
        return;
      }
      notificationService.showError('Error adding user: ' + parseError(e));
    });
  }

  function doEditUser() {
    sessionHttp.put('/api/user', $scope.theUser).then(function(resp) {
      notificationService.showSuccess('Edited user succesfully');
      refresh()
    }).catch(function(e) {
      if (!isLoggedIn()) {
        return;
      }
      notificationService.showError('Error editing user: ' + parseError(e));
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

  function init() {
    $('#add-repo-modal').on('shown.bs.modal', function () {
      $('#add-repo-repo').focus()
    });

    refresh();
  }

  function refresh() {
    return sessionHttp.get('/api/repos').then(function(resp) {
      $scope.repos = resp.data.repos;
    }).catch(function(e) {
      if (!isLoggedIn()) {
        return;
      }

      notificationService.showError('Error getting repos: ' + parseError(e));
    });
  }

  $scope.displayJIRAs = function(repo) {
    if (!repo.jira_config || repo.jira_config.length == 0) {
      return '(none)';
    } else {
      return repo.jira_config.map(function(c) { return c.jira_project; }).join(', ');
    }
  }

  $scope.editRepo = function(repo) {
    $scope.theRepo = repo;
    $('#add-repo-modal').modal('show');
  }

  $scope.addRepo = function() {
    $scope.theRepo = {
      force_push_notify: true,
      use_threads: true,
      jira_config: [],
    };
    $('#add-repo-modal').modal('show');
  }

  $scope.addRepoSubmit = function() {
    $('#add-repo-modal').modal('hide');
    var editing = !!$scope.theRepo.id;
    if (editing) {
      doEditRepo();
    } else {
      doAddRepo();
    }
  }

  $scope.addJIRA = function(theRepo) {
    theRepo.jira_config.splice(0, 0, {});
  };

  $scope.removeJIRA = function(theRepo, index) {
   theRepo.jira_config.splice(index, 1);
  }

  function doAddRepo() {
    sessionHttp.post('/api/repos', $scope.theRepo).then(function(resp) {
      notificationService.showSuccess('Added repo succesfully');
      refresh()
    }).catch(function(e) {
      if (!isLoggedIn()) {
        return;
      }
      notificationService.showError('Error adding repo: ' + parseError(e));
    });
  }

  function doEditRepo() {
    sessionHttp.put('/api/repo', $scope.theRepo).then(function(resp) {
      notificationService.showSuccess('Edited repo succesfully');
      refresh()
    }).catch(function(e) {
      if (!isLoggedIn()) {
        return;
      }
      notificationService.showError('Error editing repo: ' + parseError(e));
    });
  }

  $scope.removeRepo = function(repo) {
    if (!confirm("Are you sure you want to delete repo " + repo.repo + "?")) {
      return;
    }
    return sessionHttp.delete('/api/repo?id=' + Number(repo.id)).then(function(resp) {
      notificationService.showSuccess('Remove repo succesfully');
      refresh();
    }).catch(function(e) {
      if (!isLoggedIn()) {
        return;
      }
      notificationService.showError('Error removing repo: ' + parseError(e));
    });
  }

  // init
  init();
});

app.controller('VersionsController', function($rootScope, $scope, sessionHttp, notificationService)  {

  var jiraBase = null;

  function init() {
    $scope.reset();
    $scope.req.admin_user = loggedInUser();

    $('#auth-modal').on('shown.bs.modal', function () {
      $('#auth-password').focus()
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
      if (resp.data.login_suffix) {
        $scope.req.admin_user = loggedInUser() + resp.data.login_suffix;
      }
      if (resp.data.error) {
        notificationService.showError(resp.data.error);
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
