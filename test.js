var Jasmine = require('jasmine');
var SpecReporter = require('jasmine-spec-reporter');

var jrunner = new Jasmine();
jrunner.env.clearReporters();
jrunner.addReporter(new SpecReporter());
jrunner.loadConfigFile();
jrunner.execute();
