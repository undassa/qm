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

`porter/porter.yaml` declares the six services (core, auth, web-ui, admin, portal,
egress-proxy) as Porter apps built from `deploy/*/Dockerfile`:

```bash
porter apply -f porter/porter.yaml
```

Porter builds the images, runs the services, and assigns each web service a hostname, so
this path needs no DNS record, no TLS certificate, and no ingress controller. Set the
service secrets from the Porter dashboard or `porter apply`'s env-group support, and use
the portal's assigned hostname as `PUBLIC_WEB_URL`.

`deploy/helm/` carries the same topology as a Helm chart for operators who would rather
manage the release themselves; it expects the images published at
`ghcr.io/yc-software/qm/<service>` and an ingress you configure.

## Giving published apps stable hostnames

Apps the agent deploys are reachable only if the cluster gives them an address. Without
one, `apply` fails with `the cluster assigned no hostname`, and core warns at startup.

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
| `PORTER_DEPLOY_TTL_SEC` / `PORTER_SANDBOX_TTL_SEC` | Reap bodies after this long                                                        |

To QA a branch against a real Porter cluster before deploying it, the dev instance takes
the same backend:

```bash
bash scripts/dev-instance.sh up --sandbox porter
```
