## Credentials

### Asana Personal Access Token (PAT):

1. Go to [Asana Developer Console](https://developers.asana.com/docs/personal-access-token).
2. Click "+ New Access Token".
3. Save this token (string starting with 1/... or 2/...).

### Google OAuth2 Credentials:

1. Go to the [Google Cloud Console](https://console.cloud.google.com).
2. Create a project and enable the Google Tasks API.
3. Go to "Credentials" -> "Create Credentials" -> "OAuth Client ID".
4. Choose Desktop App.
5. Download the JSON file and rename it to client_secret.json. Place it in your project root.

## Setup (no docker)

1. Get credentials from above.
2. Go to your Asana home page and find your asana project ID. In your browser url you will see something like: `https://app.asana.com/1/SOME_NUMBER_HERE/home`. `SOME_NUMBER_HERE` will be your project ID.
3. Create a `.env` file in project root that looks something like:
```
ASANA_PAT=<YOUR_ASANA_PAT_FROM_CREDENTIALS>
PROJECT_GID=<SOME_NUMBER_HERE>
RUST_LOG=info
```

Then just run with `cargo run --release`

## Setup (docker)

1. Provide the above environment variables.
2. Provide `client_secret.json` as a docker secret, mapped to `/secret/client_secret.json`
3. Provide a docker volume for google token caching mapped to `/data`
4. Build image with `docker build -t gtasks-asana-bridge .`
