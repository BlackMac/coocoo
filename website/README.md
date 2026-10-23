# coocoo.dev

The Coocoo homepage: plain PHP (≥ 8.1), no framework, no Composer, no database. It runs on
Hetzner Webhosting (Apache + PHP). The contents of `public/` are the document root.

```
public/
  index.php changelog.php impressum.php datenschutz.php 404.php
  .htaccess        pretty URLs, 404, HTTPS + bare-domain redirect, security headers (CSP), compression
  inc/             config.php (site URL, repo, legal data, fallback release, measured numbers),
                   bootstrap.php, layout.php, github.php (GitHub API + file cache), ua.php (OS → installer)
  cache/           JSON cache written by PHP; denied to the web, never uploaded or deleted by the deploy
  assets/          css/site.css, js/site.js, fonts (Inter, JetBrains Mono; OFL), img
router.php         local php -S router that mirrors .htaccess
```

## Run locally

```sh
php -S 127.0.0.1:8080 -t website/public website/router.php
```

`php -S` ignores `.htaccess`, so the router mirrors its rewrites. To test the real Apache rules:

```sh
docker run --rm -p 127.0.0.1:8092:80 -v "$PWD/website/public:/var/www/html:ro" \
  --tmpfs /var/www/html/cache:uid=33,gid=33 php:8.4-apache sh -c \
  "a2enmod rewrite headers deflate && sed -i 's/AllowOverride None/AllowOverride All/' /etc/apache2/apache2.conf && exec apache2-foreground"
```

## GitHub data

Version, installers, stars and the changelog come from the GitHub API (`inc/github.php`), via cURL
or, without the cURL extension, PHP streams. Responses are cached in `cache/` for an hour; the
commit lists between two tags are cached permanently. A failed request (timeout 2 s) makes the site
use stale cache or the fallback in `inc/config.php` for 5 minutes without trying GitHub again, so
a page never waits on GitHub more than once. Unauthenticated, GitHub allows 60 requests per hour
per IP, which the cache stays well below. A token is optional: set `COOCOO_GITHUB_TOKEN` on the
server; never commit it.

On each release, update `fallback_release` in `inc/config.php`. When the README's resource
measurements change, update `stats` there too.

## Hetzner setup

1. konsoleH → *Services → PHP configuration*: PHP 8.4 (or the `FcgidWrapper` line in `.htaccess`).
2. Point the domain `coocoo.dev` (and `www.coocoo.dev`, which redirects) at the account and enable
   a Let's Encrypt certificate. `.htaccess` redirects to `https://coocoo.dev`.
3. The upload path is the domain's document root, typically `public_html`. `cache/` must be
   writable by PHP, which it is when uploaded by the account's own FTP user.
4. The privacy policy states that Hetzner processes data on our behalf (Art. 28 GDPR): conclude the
   data processing agreement (AV-Vertrag) in konsoleH.

## Deploy

`.github/workflows/website-deploy.yml` runs on every push to `main` that changes `website/` (and by
hand via *Run workflow*): `php -l` on all files, then `lftp mirror --reverse --delete` of
`website/public/` over SFTP. `cache/*.json` on the server is neither overwritten nor deleted.
Without the secrets the upload step only logs a warning.

Repository secrets (*Settings → Secrets and variables → Actions*):

| Secret | Value |
|---|---|
| `HETZNER_SFTP_HOST` | server name from konsoleH (*Access details*), e.g. `www123.your-server.de` |
| `HETZNER_SFTP_USER` | FTP/SFTP user |
| `HETZNER_SFTP_PASSWORD` | its password |
| `HETZNER_SFTP_PATH` | target directory, e.g. `public_html` |
| `HETZNER_SFTP_KNOWN_HOSTS` | optional, output of `ssh-keyscan <host>`; without it the host key is accepted unverified |
