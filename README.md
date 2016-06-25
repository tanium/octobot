octocat slack
=============

The built-in GitHub slack integration is not very good.
It is very chatty and does not trigger appropriate pull request notifications.
This results in code left un-reviewed -- a sad state of affairs!

This integration receives webhook events from a github repo and
transforms them into meaningful webhook events for slack.
