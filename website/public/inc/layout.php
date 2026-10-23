<?php
declare(strict_types=1);

const GITHUB_ICON = '<svg viewBox="0 0 16 16" fill="currentColor" aria-hidden="true"><path d="M8 0a8 8 0 0 0-2.53 15.59c.4.07.55-.17.55-.38v-1.33c-2.23.48-2.7-1.07-2.7-1.07-.36-.92-.89-1.17-.89-1.17-.73-.5.05-.49.05-.49.8.06 1.23.83 1.23.83.71 1.22 1.87.87 2.33.66.07-.52.28-.87.5-1.07-1.78-.2-3.65-.89-3.65-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82a7.6 7.6 0 0 1 4 0c1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.28.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48v2.2c0 .21.15.46.55.38A8 8 0 0 0 8 0z"/></svg>';

function repo_url(string $suffix = ''): string
{
    return 'https://github.com/' . site('repo') . $suffix;
}

/**
 * Sends headers and prints everything up to and including the navigation.
 *
 * @param array{title:string,description:string,path:string,lang?:string,noindex?:bool,jsonld?:string,vary_ua?:bool,body_class?:string} $page
 */
function page_head(array $page): void
{
    header('Content-Type: text/html; charset=utf-8');
    header('Cache-Control: public, max-age=300');
    if (!empty($page['vary_ua'])) {
        header('Vary: User-Agent');
    }
    $url = rtrim((string) site('site_url'), '/') . $page['path'];
    $og = rtrim((string) site('site_url'), '/') . '/og-image.png';
    $stars = gh_stars();
    $showStars = $stars !== null && $stars >= (int) site('stars_min');
    ?>
<!doctype html>
<html lang="<?= e($page['lang'] ?? 'en') ?>">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title><?= e($page['title']) ?></title>
<meta name="description" content="<?= e($page['description']) ?>">
<?php if (!empty($page['noindex'])): ?>
<meta name="robots" content="noindex">
<?php endif ?>
<link rel="canonical" href="<?= e($url) ?>">
<meta name="theme-color" content="#0b0c0e">
<meta name="color-scheme" content="dark">
<meta property="og:type" content="website">
<meta property="og:site_name" content="Coocoo">
<meta property="og:title" content="<?= e($page['title']) ?>">
<meta property="og:description" content="<?= e($page['description']) ?>">
<meta property="og:url" content="<?= e($url) ?>">
<meta property="og:image" content="<?= e($og) ?>">
<meta property="og:image:width" content="1200">
<meta property="og:image:height" content="630">
<meta name="twitter:card" content="summary_large_image">
<meta name="twitter:title" content="<?= e($page['title']) ?>">
<meta name="twitter:description" content="<?= e($page['description']) ?>">
<meta name="twitter:image" content="<?= e($og) ?>">
<link rel="icon" href="/favicon.svg" type="image/svg+xml">
<link rel="apple-touch-icon" href="/apple-touch-icon.png">
<link rel="preload" href="/assets/fonts/Inter-4.1-latin.woff2" as="font" type="font/woff2" crossorigin>
<link rel="stylesheet" href="<?= e(asset('assets/css/site.css')) ?>">
<?php if (!empty($page['jsonld'])): ?>
<script type="application/ld+json"><?= $page['jsonld'] ?></script>
<?php endif ?>
<script src="<?= e(asset('assets/js/site.js')) ?>" defer></script>
</head>
<body class="<?= e($page['body_class'] ?? '') ?>">
<svg class="sprite" aria-hidden="true" focusable="false">
  <defs>
    <linearGradient id="lg-outer" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#7a88ff"/><stop offset="1" stop-color="#4a57f0"/></linearGradient>
    <symbol id="logo" viewBox="100 100 824 824">
      <rect x="100" y="100" width="824" height="824" rx="186" fill="#16181f"/>
      <g fill="none" stroke-linecap="round"><path d="M688.78 335.22A250 250 0 1 0 688.78 688.78" stroke="url(#lg-outer)" stroke-width="84"/><path d="M603.92 420.08A130 130 0 1 0 603.92 603.92" stroke="#aab3ff" stroke-width="64"/></g>
      <circle cx="716" cy="512" r="38" fill="#fff"/>
    </symbol>
  </defs>
</svg>
<a class="skip" href="#main">Skip to content</a>
<nav class="top" aria-label="Main">
  <div class="wrap">
    <a class="brand" href="/"><svg aria-hidden="true"><use href="#logo"/></svg>Coocoo</a>
    <div class="links">
      <a href="/#queue">Product</a><a href="/#fleet">Fleet</a><a href="/#ram">Resources</a><a href="/#start">Get started</a><a href="/changelog">Changelog</a>
    </div>
    <div class="right">
      <a class="btn" href="<?= e(repo_url()) ?>" aria-label="Coocoo on GitHub<?= $showStars ? ', ' . (int) $stars . ' stars' : '' ?>"><?= GITHUB_ICON ?><?php if ($showStars): ?><span class="star">★ <?= e(fmt_count((int) $stars)) ?></span><?php endif ?></a>
      <a class="btn primary" href="/#download">Download</a>
    </div>
  </div>
</nav>
<?php
}

function page_foot(): void
{
    ?>
<footer class="foot">
  <div class="wrap">
    <a class="brand" href="/"><svg aria-hidden="true"><use href="#logo"/></svg>Coocoo</a>
    <span>MIT · Made by <a href="<?= e(site('legal')['website']) ?>">YuzuHub</a> in Düsseldorf</span>
    <div class="r">
      <a href="/changelog">Changelog</a><a href="<?= e(repo_url()) ?>">GitHub</a><a href="/impressum">Impressum</a><a href="/datenschutz">Datenschutz</a>
    </div>
  </div>
</footer>
</body>
</html>
<?php
}
