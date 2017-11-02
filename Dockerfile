FROM ubuntu:latest

# install run deps
RUN apt-get update \
  && apt-get install -y \
     ca-certificates \
     git \
     firejail \
     gosu \
     python \
     openssl \
  && rm -fr /var/lib/apt/lists/

RUN groupadd -r octobot
RUN useradd -r -g octobot -m -s /sbin/nologin octobot

ENV HOME=/home/octobot

RUN mkdir -p $HOME/bin
RUN mkdir -p $HOME/logs

ADD ./.docker-tmp/bin $HOME/bin
ADD docker-entrypoint.sh $HOME/bin/

RUN chown -R octobot:octobot $HOME/bin

VOLUME /data
WORKDIR $HOME/bin

EXPOSE 3000

ENV USER=octobot
ENV RUST_LOG=info

ENV PATH=$PATH:$HOME/bin
ENV GIT_AUTHOR_NAME octobot
ENV GIT_AUTHOR_EMAIL octobot@tanium.com
ENV GIT_COMMITTER_NAME $GIT_AUTHOR_NAME
ENV GIT_COMMITTER_EMAIL $GIT_AUTHOR_EMAIL

ENTRYPOINT ["docker-entrypoint.sh"]
CMD ["octobot", "/data/config.toml"]
