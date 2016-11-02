const DND_MARKER = 'DO NOT DISTURB';
var users = require('./users');

exports.desiresPeaceAndQuiet = function(text) {
    return text === DND_MARKER || text === users.mention(DND_MARKER);
}
