FROM node:12.18.3-buster-slim

ADD https://github.com/krallin/tini/releases/download/v0.19.0/tini /tini
RUN chmod +x /tini

RUN npm install --global ssb-server@16.0.1 --no-shrinkwrap && rm -rf /root/.npm

ENTRYPOINT [ "/tini", "--", "ssb-server" ]
CMD [ "start" ]
