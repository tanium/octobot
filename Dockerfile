FROM ubuntu:latest

# install run deps
RUN apt-get update \
  && apt-get install -y ca-certificates git firejail \
  && rm -fr /var/lib/apt/lists/

RUN groupadd -r octobot
RUN useradd -r -g octobot -m -s /sbin/nologin octobot

ENV HOME=/home/octobot

RUN mkdir -p $HOME/bin
RUN mkdir -p $HOME/logs

ADD ./.docker-tmp/bin $HOME/bin

RUN chown -R octobot:octobot $HOME/bin

USER octobot
VOLUME /data
WORKDIR $HOME/bin

EXPOSE 3000

ENV USER=octobot
ENV RUST_LOG=info

CMD  $HOME/bin/octobot /data/config.toml
