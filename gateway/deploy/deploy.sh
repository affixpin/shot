#!/bin/bash
# One-shot provisioner. Idempotent: re-running is safe.
#
# Prereqs:
#   gcloud auth login
#   gcloud services enable compute.googleapis.com secretmanager.googleapis.com iam.googleapis.com
#
# After the VM boots, add the secret values:
#   echo -n "YOUR_TELEGRAM_TOKEN" | gcloud secrets versions add telegram-token --data-file=-
#   echo -n "YOUR_GEMINI_KEY"     | gcloud secrets versions add gemini-api-key --data-file=-

set -euo pipefail

PROJECT="${PROJECT:-$(gcloud config get-value project)}"
ZONE="${ZONE:-us-central1-a}"
NAME="${NAME:-shot-bot}"
SA_EMAIL="$NAME@$PROJECT.iam.gserviceaccount.com"

# Secrets (empty placeholders — fill in afterwards with `gcloud secrets versions add`)
for secret in telegram-token gemini-api-key jina-api-key; do
  if ! gcloud secrets describe "$secret" >/dev/null 2>&1; then
    gcloud secrets create "$secret" --replication-policy=automatic
  fi
done

# Service account
if ! gcloud iam service-accounts describe "$SA_EMAIL" >/dev/null 2>&1; then
  gcloud iam service-accounts create "$NAME" --display-name="shot bot VM"
fi

# Grant secretmanager.secretAccessor for each secret
for secret in telegram-token gemini-api-key jina-api-key; do
  gcloud secrets add-iam-policy-binding "$secret" \
    --member="serviceAccount:$SA_EMAIL" \
    --role=roles/secretmanager.secretAccessor \
    --condition=None >/dev/null
done

# VM (e2-small: 2GB RAM, ~$14/mo — roomy for the gateway + Caddy + per-chat shot containers)
HERE=$(dirname "$0")
if ! gcloud compute instances describe "$NAME" --zone="$ZONE" >/dev/null 2>&1; then
  gcloud compute instances create "$NAME" \
    --zone="$ZONE" \
    --machine-type=e2-small \
    --image-family=debian-12 \
    --image-project=debian-cloud \
    --service-account="$SA_EMAIL" \
    --scopes=cloud-platform \
    --tags=http-server,https-server \
    --metadata-from-file=startup-script="$HERE/startup.sh"
fi

echo
echo "VM created. Add the secret values, then the startup script will pick them up on the next boot:"
echo
echo "  echo -n 'YOUR_TELEGRAM_TOKEN' | gcloud secrets versions add telegram-token --data-file=-"
echo "  echo -n 'YOUR_GEMINI_KEY'     | gcloud secrets versions add gemini-api-key --data-file=-"
echo
echo "Then trigger a reboot (or re-run startup) to make the bot come up:"
echo "  gcloud compute instances reset $NAME --zone=$ZONE"
echo
echo "Tail logs with:"
echo "  gcloud compute ssh $NAME --zone=$ZONE -- sudo journalctl -u shot-bot -f"
