
var Slack = require('node-slack');

function initSlack() {
    if (process.env.HOOK_URL) {
        hookURL = process.env.HOOK_URL;
    } else {
        console.log("Must configure HOOK_URL!");
        process.exit(1);
    }
    return new Slack(hookURL);
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
