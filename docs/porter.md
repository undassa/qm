# Running qm on Porter

[Porter](https://porter.run) provisions and manages a Kubernetes cluster inside your own
AWS, GCP, or Azure account. qm can use it two ways, independently:

- `SANDBOX_BACKEND=porter` — every agent computer is a Porter sandbox with a persistent
  home volume.
- `DEPLOY_PROVIDER=porter` — apps the agent publishes run on the same cluster.

Both need a Porter API token and the project and cluster that own the sandbox API:

```bash
PORTER_DEPLOY_API_TOKEN=<api token from Settings -> API tokens>
PORTER_DEPLOY_PROJECT_ID=<project id>
PORTER_DEPLOY_CLUSTER_ID=<cluster id>
```

## Hosting the qm surfaces

`porter/apps/` declares the six services (core, auth, web-ui, admin, portal,
egress-proxy) as one Porter app per file, built from `deploy/*/Dockerfile` — `porter
apply` takes exactly one app per invocation and silently ignores extra YAML documents,
which is why they are separate files:

```bash
for f in porter/apps/*.yaml; do porter apply -f "$f"; done
```

Porter runs the services and assigns each **public** web service an `onporter.run`
hostname with a Let's Encrypt certificate, so this path needs no DNS record, no TLS
certificate, and no ingress controller. Only the portal is meant to be Internet-facing;
auth, web-ui, and admin carry `private: true` so Porter creates no ingress for them and
they are reachable only at their in-cluster address.

Two things `porter apply` will not do for you:

- **Building on an Apple Silicon machine fails.** `build.method: docker` builds locally
  for `linux/amd64` and the legacy builder cannot cross-compile: the apply dies with
  `image ... does not provide the specified platform (linux/amd64)`. Build and push with
  `docker buildx build --platform linux/amd64 --push`, then point the app at the pushed
  image with an `image: {repository, tag}` block, or use `porter apply --remote`.
- **Env groups created by `porter env create` are not visible to `porter apply`.** Both
  the `envGroups:` key and `--attach-env-groups` fail with `internal: unable to find
latest environment with provided name`, because `porter env create` makes a
  project-scoped group and apply resolves cluster-scoped ones. Put non-secret wiring in
  the app's `env:` block and pass secrets with `--secrets KEY=value` (note that pflag
  splits those on commas, so a JSON value like `AUTH_SIGNING_JWK` has to go in `env:`).

Nothing generates the inter-service wiring for you — `cli/src/services.ts` has `fly` and
`docker` targets but no Porter one — so set it by hand on each app:

| Service      | Wiring                                                                                                                                                                                                                                                                  |
| ------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| all but core | `CORE_API_URL=http://qm-core-core.<namespace>.svc.cluster.local:8080`, `CORE_ORG_ID`, `CORE_SIGNING_SECRET`, `PORTAL_IDENTITY_SECRET`                                                                                                                                   |
| core         | `ORG_ID`, `MODEL_PROVIDER`, `HARNESS`, `SANDBOX_BACKEND`, `PUBLIC_WEB_URL`/`WEB_UI_PUBLIC_URL` (the portal's hostname), `CAPABILITY_SECRET`, `CONNECTOR_SECRET_KEY`, `SKILL_SIGNING_SECRET`, and `DATABASE_URL` when `SESSION_STORE`/`RUN_STORE` are `postgres`         |
| portal       | `PORTAL_PUBLIC_URL`, `PORTAL_SESSION_SECRET`, `WEB_UI_UPSTREAM`, `ADMIN_UPSTREAM`, `AUTH_BROKER_UPSTREAM`, `AUTH_BROKER_PREFIX=/idp`, and the `OIDC_*` set from `brokerWiring` in `cli/src/services.ts`                                                                 |
| auth         | `AUTH_ISSUER=<portal>/idp`, `AUTH_CLIENT_ID`, `AUTH_CLIENT_SECRET`, `AUTH_REDIRECT_URI=<portal>/auth/callback`, `AUTH_TOKEN_SECRET`, `AUTH_SIGNING_JWK` (a P-256 private JWK), and a `RESEND_API_KEY` or SMTP credentials — without a mail transport nobody can sign in |

The portal's hostname is only knowable after its first apply, and core, portal, and auth
all need it, so expect to apply twice: once to mint the hostname, once to wire it in.

Porter's own Postgres datastores are created from the dashboard. An RDS instance in the
cluster's VPC works too, but Postgres 17 defaults to `rds.force_ssl=1`, so the URL needs
`?sslmode=no-verify` (or a CA pinned with `DATABASE_CA_CERT`) or core dies on `no pg_hba.conf
entry ... no encryption`.

`deploy/helm/` carries the same topology as a Helm chart for operators who would rather
manage the release themselves; it expects the images published at
`ghcr.io/yc-software/qm/<service>` and an ingress you configure. Pin `image.tag` to a
commit that actually carries the Porter backend — the published tags are commit SHAs, and
one from before Porter support landed rejects `SANDBOX_BACKEND=porter` at startup.

## Giving published apps stable hostnames

Apps the agent deploys are reachable only if the cluster gives them an address. Without
one, `apply` fails with `the cluster assigned no hostname`, and core warns at startup.

The address comes from Porter's **sandbox ingress**, which is separate from the ingress
that serves Porter apps and is enabled per cluster from the dashboard. A cluster without
it rejects every publish before DNS is ever consulted, with
`HTTP 400: validation error: visibility: no private sandbox ingress is configured on this
cluster` (`no public sandbox ingress` when `PORTER_DEPLOY_VISIBILITY=public`). Turn
sandbox ingress on first; the wildcard record below is what gives the resulting apps
stable names.

Point a wildcard DNS record at the cluster's ingress load balancer and name it:

```bash
PORTER_DEPLOY_APPS_DOMAIN=apps.example.com   # *.apps.example.com -> cluster ingress
```

Each deployment then publishes at `<name>.apps.example.com`. Deployments are **private**
by default, matching the other providers; `PORTER_DEPLOY_VISIBILITY=public` opts a
deployment's domain into public ingress.

## Other knobs

| Variable                                           | Meaning                                                                            |
| -------------------------------------------------- | ---------------------------------------------------------------------------------- |
| `PORTER_DEPLOY_URL`                                | Porter API host, when it isn't `https://dashboard.porter.run`                      |
| `PORTER_SANDBOX_IMAGE`                             | Image agent computers boot from                                                    |
| `PORTER_DEPLOY_RUNNER_IMAGE`                       | Image published apps boot from (defaults to the sandbox image)                     |
| `PORTER_SANDBOX_EGRESS_PROXY_URL`                  | Forces sandbox traffic through the egress proxy; unset means no egress enforcement |
| `PORTER_SANDBOX_NAME_PREFIX`                       | Prefix for sandbox and app names on the cluster                                    |
| `PORTER_DEPLOY_VISIBILITY`                         | `public` puts a published app on public ingress; default is private                |
| `PORTER_DEPLOY_TTL_SEC` / `PORTER_SANDBOX_TTL_SEC` | Reap bodies after this long                                                        |

To QA a branch against a real Porter cluster before deploying it, the dev instance takes
the same backend:

```bash
bash scripts/dev-instance.sh up --sandbox porter
```
