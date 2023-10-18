#!/usr/bin/env python3

import os
import re
import shutil
import subprocess

is_travis = os.environ.get('TRAVIS') is not None
class task:
    def __init__(self, title):
        self.title = title
    def __enter__(self):
        if is_travis:
            print("travis_fold:start:{}".format(self.title))
        print(">> Starting {}".format(self.title))
        return self
    def __exit__(self, type, value, traceback):
        print(">> Ending {}".format(self.title))
        if is_travis:
            print("travis_fold:end:{}".format(self.title))

def run(cmd, ignore_fail=False, quiet=False):
    print(">> {}".format(cmd))
    stdout = subprocess.PIPE if quiet else None
    proc = subprocess.Popen(re.split('\s+', cmd), stdout=stdout, stderr=subprocess.STDOUT)
    _, _ = proc.communicate()
    retcode = proc.poll()
    if retcode and not ignore_fail:
        raise subprocess.CalledProcessError(retcode, cmd)

docker_tmp = './.docker-tmp'
docker_out = './.docker-tmp/bin'
if os.path.exists(docker_tmp):
    shutil.rmtree(docker_tmp)
os.makedirs(docker_out)

with task("Dockerfile.build"):
    run("docker build . -f Dockerfile.build -t octobot:build")
with task("extract_files"):
    run("docker rm -f extract", ignore_fail=True, quiet=True)
    run("docker create --name extract octobot:build")
    run("docker cp extract:/usr/src/app/target/release/octobot {}".format(docker_out))
    run("docker cp extract:/usr/src/app/target/release/octobot-passwd {}".format(docker_out))
    run("docker cp extract:/usr/src/app/target/release/octobot-ask-pass {}".format(docker_out))
    run("docker rm -f extract")
    # write out the version file
    commit_hash = subprocess.check_output(['git', 'rev-parse', '--short', 'HEAD'])
    with open(os.path.join(docker_out, 'version'), 'wb') as f:
        f.write(commit_hash)
with task("Dockerfile"):
    run("docker build . -t octobot:latest")
