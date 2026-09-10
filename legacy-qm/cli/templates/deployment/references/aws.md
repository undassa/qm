# AWS deployment

Use this after the choices and billing confirmation in `deployment.md`.
Terraform state, credentials, and every resource must belong to the operator.

## Preflight

Require Terraform, Docker, authenticated AWS credentials, two available AZs,
and AWS CLI support for Lambda MicroVMs:

```bash
aws --profile <profile> sts get-caller-identity
aws --profile <profile> ec2 describe-availability-zones --region <region> \
  --filters Name=state,Values=available
aws --profile <profile> lambda-microvms list-microvm-images --region <region>
terraform version
docker buildx version
```

If Lambda MicroVMs are unavailable, stop before mutation and offer Fly.io. Set
the account, region, service coordinates, and an operator-owned GitHub
repository and exact branch in the generated config and Terraform variables.
Never trust the upstream MH repository.

Configure a private encrypted Terraform backend, then:

```bash
npm exec mh -- infra render
terraform -chdir=infra init
terraform -chdir=infra plan -out=mh.tfplan
terraform -chdir=infra apply mh.tfplan
```

Set `publicUrl`, `env.core.AWS_PUBLIC_ORIGIN_URL`, and `aws.deployRoleArn` from
the Terraform outputs. Finish `npm exec mh -- setup .`, render again, and apply.

## Publish the agent computer and deploy

```bash
npm exec mh -- infra build-image
npm exec mh -- check
npm exec mh -- secrets push
npm exec mh -- doctor
npm exec mh -- plan
npm exec mh -- up --yes
npm exec mh -- check --live
```

Existing deployments created before private session canaries must rerun
`npm exec mh -- infra render`, review the Terraform plan, and apply it with
infrastructure-administrator credentials before enabling `check --live`. This
adds the deploy role's stack-scoped permission to run and inspect the one-off
core canary task.

The package image manifest supplies first-party control-plane images. The AWS
backend transfers them into deployment-owned ECR and records immutable digests.
After the first successful deployment, rerun `npm exec mh -- up --yes` and
confirm it reconciles the same stack.

## Agent-computer proof

Copy the exact personal scope id shown for the signed-in administrator in
Admin, then derive the same opaque storage key as the runtime and read only the
proof file from the deployment-owned S3 home snapshot:

```bash
scope_id='personal:<exact-admin-principal>'
scope_key="$(npm exec mh -- proof scope-key "$scope_id")"
bucket="$(terraform -chdir=infra output -raw object_store_bucket)"
aws --profile <profile> --region <region> s3 cp \
  "s3://$bucket/sandbox-home/$scope_key.tar" - |
  tar -xOf - workspace/mh-computer-proof.txt
```

Require the output to match the UUID created in the browser. A missing or
ambiguous scope, snapshot, or file is a failed proof.

Routine operations:

```bash
npm exec mh -- status
npm exec mh -- logs core --follow
npm exec mh -- rollback --to <release-label-or-manifest-id>
npm exec mh -- down
```

Terraform destroy is separate and destructive. Decide how to retain RDS
snapshots, S3 objects, and secrets before following the generated `AGENTS.md`
teardown section.
