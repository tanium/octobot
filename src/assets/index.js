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
      sessionStorage['session'] = '1234';

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

  $scope.users = [];
  $scope.repos = [];

  http_get('/api/users').then(function(resp) {
    $scope.users = resp.data.users;

  }).catch(function(e) {
    alert("Error getting users: " + e);
  });

  http_get('/api/repos').then(function(resp) {
    $scope.repos = resp.data.repos;

  }).catch(function(e) {
    alert("Error getting repos: " + e);
  });

  $scope.logout = function() {
    http_post('/auth/logout', {}).finally(function() {
      delete sessionStorage['session'];
    });
  }
});
