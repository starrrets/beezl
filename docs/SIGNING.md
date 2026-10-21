# Code signing setup

CI signs releases through [SignPath](https://signpath.org) (free for open
source). Until the repository variables below exist, releases are published
unsigned and CI skips the signing steps.

## 1. Apply

Apply at <https://signpath.org/apply> with this repository. SignPath
Foundation checks that the project is open source (MIT), actively maintained,
already has a release, and has some reputation, so apply after a few releases
and some stars or downloads. When accepted, add the "Code signing policy"
section they require to the README (attribution, team roles, privacy:
beezl sends no data).

## 2. Configure SignPath

In the SignPath web app:

1. **Trusted build system**: link the predefined *GitHub.com* system to the
   project.
2. **Artifact configuration** (set as default). GitHub delivers artifacts as
   ZIP, and each signing request holds one exe:

   ```xml
   <?xml version="1.0" encoding="utf-8"?>
   <artifact-configuration xmlns="http://signpath.io/artifact-configuration/v1">
     <zip-file>
       <pe-file path="*.exe">
         <authenticode-sign/>
       </pe-file>
     </zip-file>
   </artifact-configuration>
   ```

3. **Signing policy**: `release-signing`, with yourself as approver. Each
   release then waits for one approval click in SignPath (two requests: the
   app, then the installer that embeds it).
4. Create a CI user and an API token for it, with submitter rights on the
   policy.

## 3. Configure GitHub

Repository **Settings → Secrets and variables → Actions**:

| Kind     | Name                       | Value                        |
|----------|----------------------------|------------------------------|
| Secret   | `SIGNPATH_API_TOKEN`       | the CI user's API token      |
| Variable | `SIGNPATH_ORGANIZATION_ID` | organization id from SignPath |
| Variable | `SIGNPATH_PROJECT_SLUG`    | e.g. `beezl`                 |
| Variable | `SIGNPATH_POLICY_SLUG`     | e.g. `release-signing`       |

The next version bump on `main` produces a signed release. Then update the
"Code signing" sections in `README.md` and `README.ru.md`.

## Alternatives

* **Certum Open Source Code Signing**: a cheap certificate for individual
  open-source developers, available worldwide. The key lives in their cloud
  HSM (SimplySign), which is awkward to automate in CI.
* **Azure Artifact Signing**: about $10/month and works well with CI, but
  individuals qualify only in the USA and Canada.

Signing removes the "unknown publisher" text. SmartScreen can still warn until
the signed downloads build some reputation.
