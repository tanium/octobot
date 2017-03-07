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
    $http.post('/login', {
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

  $scope.logout = function() {
    $http.post('/logout', {}, {
      headers: {
        session: sessionStorage['session'],
      }
    }).finally(function() {
      delete sessionStorage['session'];
    });
  }
});
