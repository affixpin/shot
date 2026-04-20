FROM node:22-alpine

# docker CLI so gateway can spawn shot containers via the mounted daemon socket
RUN apk add --no-cache docker-cli

WORKDIR /app
COPY package.json ./
RUN npm install --include=dev
COPY bot-server.ts ./

ENV DATA_DIR=/data
ENV SHOT_TEMPLATE_DIR=/srv/shot-template
# Pin the Docker API version the client speaks so it works against
# older daemons (Debian's docker.io exposes API 1.41). Without this
# the CLI refuses to talk to the daemon on version mismatch.
ENV DOCKER_API_VERSION=1.41
VOLUME ["/data"]

ENTRYPOINT ["npm", "start"]
