var repos = require('../lib/repos');

describe('repos', function() {

   describe("getChannel", function() {

      afterEach(function() {
          repos.setReposMapForTesting({});
      });

      it("should return the mapped channel for the given repo name", function() {
          repos.setReposMapForTesting({
            'git.company.org': {
                'some-org/some-repo': 'some-repo-specific',
                'some-org': 'some-org-default'
            }
          });
          expect(repos.getChannel({
              html_url: 'https://git.company.org/blah',
              full_name: 'some-org/some-repo', owner: { login: 'some-org' }
          })).toEqual('some-repo-specific');
      });

      it("should return the org channel if no specific repo is given", function() {
          repos.setReposMapForTesting({
            'git.company.org': {
                'some-org/other-repo': 'other-repo-specific',
                'some-org': 'some-org-default'
            }
          });
          expect(repos.getChannel({
              html_url: 'https://git.company.org/blah',
              full_name: 'some-org/some-repo', owner: { login: 'some-org' }
          })).toEqual('some-org-default');
      });

      it("should return null for no defined repos", function() {
          expect(repos.getChannel({
              html_url: 'https://git.company.org/blah',
              full_name: 'some-org/some-repo', owner: { login: 'some-org' }
          })).toBeNull();
      });
   });
});
