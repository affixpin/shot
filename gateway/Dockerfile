FROM node:22-alpine

# docker CLI so gateway can spawn shot containers via the mounted daemon socket
RUN apk add --no-cache docker-cli

WORKDIR /app
COPY package.json ./
RUN npm install --include=dev
COPY bot-server.ts ./

ENV DATA_DIR=/data
VOLUME ["/data"]

ENTRYPOINT ["npm", "start"]
