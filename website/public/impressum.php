<?php
declare(strict_types=1);
require __DIR__ . '/inc/bootstrap.php';

$l = site('legal');

page_head([
    'title' => 'Impressum — Coocoo',
    'description' => 'Impressum der Website coocoo.dev.',
    'path' => '/impressum',
    'lang' => 'de',
    'noindex' => true,
    'body_class' => 'subpage',
]);
?>
<main id="main" class="page wrap legal">
  <header class="page-h">
    <p class="eyebrow">IMPRESSUM</p>
    <h1>Impressum</h1>
  </header>

  <h2>Angaben gemäß § 5 DDG</h2>
  <p><?= e($l['company']) ?><br><?= e($l['street']) ?><br><?= e($l['city']) ?><br><?= e($l['country']) ?></p>

  <h2>Kontakt</h2>
  <p>Telefon: <a href="tel:<?= e(preg_replace('/[^+\d]/', '', str_replace('(0)', '', $l['phone']))) ?>"><?= e($l['phone']) ?></a><br>
  E-Mail: <a href="mailto:<?= e($l['email']) ?>"><?= e($l['email']) ?></a><br>
  Website: <a href="<?= e($l['website']) ?>"><?= e(preg_replace('#^https?://|/$#', '', $l['website'])) ?></a></p>

  <h2>Vertretungsberechtigte Geschäftsführung</h2>
  <p><?= e($l['managing_directors']) ?></p>

  <h2>Registereintrag</h2>
  <p><?= e($l['register']) ?></p>

  <h2>Umsatzsteuer-ID</h2>
  <p>Umsatzsteuer-Identifikationsnummer gemäß § 27a UStG: <?= e($l['vat_id']) ?></p>

  <h2>Verbraucherstreitbeilegung</h2>
  <p>Wir sind nicht bereit und nicht verpflichtet, an Streitbeilegungsverfahren vor einer Verbraucherschlichtungsstelle teilzunehmen.</p>

  <h2>Software</h2>
  <p>Coocoo ist freie Software unter der MIT-Lizenz. Der Quellcode liegt auf <a href="<?= e(repo_url()) ?>">GitHub</a>.</p>

  <p class="legal-links"><a href="/datenschutz">Datenschutzerklärung →</a></p>
</main>
<?php page_foot();
