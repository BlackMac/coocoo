<?php
declare(strict_types=1);
require __DIR__ . '/inc/bootstrap.php';

http_response_code(404);
page_head([
    'title' => 'Not found — Coocoo',
    'description' => 'This page does not exist.',
    'path' => '/404',
    'noindex' => true,
    'body_class' => 'subpage',
]);
?>
<main id="main" class="page wrap notfound">
  <p class="eyebrow">404</p>
  <h1>Nothing here.<br><span class="dim">No agent is waiting on this page.</span></h1>
  <div class="cta">
    <a class="btn primary lg" href="/">Back to the overview</a>
    <a class="btn lg" href="/changelog">Changelog</a>
  </div>
</main>
<?php page_foot();
