# ZITADEL identity setup

Codasaurus can use ZITADEL as its identity provider. In this mode, ZITADEL owns
passwords, MFA, passkeys, identity-provider federation, users, and invitations;
Codasaurus keeps only the application-side identity link and authorization data.

## Local Compose

1. Copy `.env.example` to `.env` and replace the local ZITADEL master key and
   first-admin password.
2. Start the identity profile:

   ```bash
   docker compose --profile identity up -d postgres redis zitadel
   ```

3. Open the ZITADEL console at
   `http://host.docker.internal:8080/ui/console?login_hint=zitadel-admin@zitadel.localhost`.
4. Create a project named `Codasaurus`, then create a web application using
   Authorization Code with PKCE. Add this redirect URI:

   ```text
   http://localhost:3000/api/auth/oidc/callback
   ```

5. Set the resulting client ID and secret in `.env`:

   ```dotenv
   CODASAURUS_AUTH_MODE=zitadel
   ZITADEL_ISSUER=http://host.docker.internal:8080
   ZITADEL_CLIENT_ID=...
   ZITADEL_CLIENT_SECRET=...
   ```

6. Start Codasaurus. The dashboard login page will show only the ZITADEL sign-in
   flow. Local password login and Codasaurus-local invitations return `410 Gone`.

The default Compose ZITADEL service is intended for local development only. For
deployment, set `ZITADEL_EXTERNALDOMAIN`, `ZITADEL_EXTERNALSECURE=true`, TLS at the
reverse proxy, and a stable external issuer URL. ZITADEL's public URL settings
must exactly match the URL users access.

## Ownership boundary

ZITADEL is the source of truth for identity and organization membership. The
Rust service remains the source of truth for repository access, review policy,
budgets, GitHub installations, and audit records. Do not authorize a repository
operation solely from a stale token claim; resolve the current Codasaurus
organization/team mapping before allowing the operation.

