<?php
declare(strict_types=1);
require __DIR__ . '/inc/bootstrap.php';

$releases = gh_releases();
$compares = gh_compares($releases);
const COMMITS_SHOWN = 30;

page_head([
    'title' => 'Changelog — Coocoo',
    'description' => 'Every Coocoo release with its installers and the changes since the previous version.',
    'path' => '/changelog',
    'body_class' => 'subpage',
]);
?>
<main id="main" class="page wrap">
  <header class="page-h">
    <p class="eyebrow">CHANGELOG</p>
    <h1>What changed.</h1>
    <p class="big">Releases from <a href="<?= e(repo_url('/releases')) ?>">GitHub</a>, newest first.</p>
    <?php if (!empty($releases[0]['fallback'])): ?>
    <p class="more">The release history can't be loaded right now. The full list is on <a href="<?= e(repo_url('/releases')) ?>">GitHub</a>.</p>
    <?php endif ?>
  </header>

  <ol class="releases">
    <?php foreach ($releases as $i => $r): ?>
    <?php $cmp = $compares[$r['tag']] ?? null; $isOldest = $i === count($releases) - 1 && empty($r['fallback']); ?>
    <li class="release" id="<?= e($r['tag']) ?>">
      <div class="rel-side">
        <a class="rel-tag" href="#<?= e($r['tag']) ?>"><?= e($r['tag']) ?></a>
        <time datetime="<?= e($r['published_at']) ?>"><?= e(fmt_date($r['published_at'])) ?></time>
        <?php if ($i === 0 && !$r['prerelease']): ?><span class="rel-badge">latest</span><?php endif ?>
        <?php if ($r['prerelease']): ?><span class="rel-badge pre">pre-release</span><?php endif ?>
      </div>
      <div class="rel-main">
        <h2><a href="<?= e($r['html_url']) ?>"><?= e($r['name']) ?></a></h2>
        <?php if ($r['body_html'] !== ''): ?>
        <div class="rel-body"><?= $r['body_html'] /* rendered and sanitized by GitHub */ ?></div>
        <?php endif ?>

        <?php if ($cmp !== null && $cmp['commits'] !== []): ?>
        <h3 class="rel-sub">Changes since <?= e($releases[$i + 1]['tag']) ?></h3>
        <ul class="commits">
          <?php foreach (array_slice($cmp['commits'], 0, COMMITS_SHOWN) as $c): ?>
          <li><a class="sha" href="<?= e($c['url']) ?>"><?= e($c['sha']) ?></a> <?= e($c['subject']) ?></li>
          <?php endforeach ?>
        </ul>
        <?php if (count($cmp['commits']) > COMMITS_SHOWN || $cmp['total'] > count($cmp['commits'])): ?>
        <p class="more"><a href="<?= e($cmp['url']) ?>">All <?= (int) $cmp['total'] ?> commits on GitHub →</a></p>
        <?php endif ?>
        <?php elseif ($isOldest): ?>
        <p class="rel-first">First public release.</p>
        <?php elseif ($cmp === null): ?>
        <p class="more"><a href="<?= e(repo_url('/compare/' . rawurlencode($releases[$i + 1]['tag']) . '...' . rawurlencode($r['tag']))) ?>">Changes since <?= e($releases[$i + 1]['tag']) ?> on GitHub →</a></p>
        <?php endif ?>

        <?php if ($r['assets'] !== []): ?>
        <h3 class="rel-sub">Downloads</h3>
        <ul class="assets">
          <?php foreach ($r['assets'] as $a): ?>
          <li><a href="<?= e($a['url']) ?>"><?= e($a['name']) ?></a><?php if ($a['size'] > 0): ?> <span><?= e(fmt_size($a['size'])) ?></span><?php endif ?></li>
          <?php endforeach ?>
        </ul>
        <?php endif ?>
      </div>
    </li>
    <?php endforeach ?>
  </ol>
</main>
<?php page_foot();
