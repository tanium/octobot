FROM ubuntu:latest

ENV DEBIAN_FRONTEND=noninteractive

# install run deps
RUN apt-get update \
  && apt-get install -y \
     ca-certificates \
     git \
     firejail \
     gosu \
     python3.6 \
     openssl \
     libsqlite3-0 \
     libldap2-dev \
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

ENTRYPOINT ["docker-entrypoint.sh"]
CMD ["octobot", "/data/config.toml"]
