
var fs = require('fs');
var Slack = require('node-slack');

function initSlack() {
    var hookFile = '.secret-hook.txt';
    if (!fs.existsSync(hookFile)) {
        console.log("Hook file does not exist: " + hookFile);
        process.exit(1);
    }
    var hook = fs.readFileSync(hookFile).toString().trim();
    return new Slack(hook);
}


function main()  {
    var slack = initSlack();

    slack.send({
      text: 'Howdy!',
      channel: '@matt.hauck',
    });
}


if (require.main === module) {
    main();
}
