#!/usr/bin/env python

import os
import re
import shutil
import subprocess

def run(cmd, ignore_fail=False, quiet=False):
    print ">> {}".format(cmd)
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

run("docker build . -f Dockerfile.build -t octobot:build")
run("docker run --privileged --rm octobot:build")
run("docker rm -f extract", ignore_fail=True, quiet=True)
run("docker create --name extract octobot:build")
run("docker cp extract:/usr/src/app/target/release/octobot {}".format(docker_out))
run("docker cp extract:/usr/src/app/target/release/octobot-passwd {}".format(docker_out))
run("docker cp extract:/usr/src/app/target/release/octobot-ask-pass {}".format(docker_out))
run("docker rm -f extract")
run("docker build . -t octobot:latest")
