<?php
declare(strict_types=1);
require __DIR__ . '/inc/bootstrap.php';

$l = site('legal');

page_head([
    'title' => 'Datenschutzerklärung — Coocoo',
    'description' => 'Datenschutzerklärung der Website coocoo.dev.',
    'path' => '/datenschutz',
    'lang' => 'de',
    'noindex' => true,
    'body_class' => 'subpage',
]);
?>
<main id="main" class="page wrap legal">
  <header class="page-h">
    <p class="eyebrow">DATENSCHUTZ</p>
    <h1>Datenschutzerklärung</h1>
    <p class="big">Kurz gesagt: Diese Website setzt keine Cookies, nutzt kein Tracking und keine Analyse-Dienste und lädt keine Inhalte von fremden Servern. Personenbezogene Daten fallen nur in den Server-Logfiles des Hosters an.</p>
  </header>

  <h2>1. Verantwortlicher</h2>
  <p><?= e($l['company']) ?><br><?= e($l['street']) ?>, <?= e($l['city']) ?>, <?= e($l['country']) ?><br>
  E-Mail: <a href="mailto:<?= e($l['privacy_email']) ?>"><?= e($l['privacy_email']) ?></a><br>
  Vertreten durch die Geschäftsführung: <?= e($l['managing_directors']) ?></p>

  <h2>2. Hosting und Server-Logfiles</h2>
  <p>Die Website wird bei der Hetzner Online GmbH, Industriestr. 25, 91710 Gunzenhausen, auf Servern in Deutschland betrieben. Hetzner verarbeitet die Daten in unserem Auftrag (Art. 28 DSGVO).</p>
  <p>Bei jedem Aufruf speichert der Webserver automatisch: IP-Adresse, Datum und Uhrzeit, aufgerufene Adresse, Referrer (die zuvor besuchte Seite), Browser und Betriebssystem (User-Agent), HTTP-Statuscode und übertragene Datenmenge.</p>
  <p>Zweck ist die Auslieferung der Website, die Sicherheit des Betriebs und die Abwehr von Angriffen. Rechtsgrundlage ist Art. 6 Abs. 1 lit. f DSGVO; unser berechtigtes Interesse liegt im sicheren und fehlerfreien Betrieb. Die Logfiles werden nur so lange gespeichert, wie es für diese Zwecke erforderlich ist, und danach gelöscht.</p>
  <p>Die Verbindung ist per TLS (HTTPS) verschlüsselt.</p>

  <h2>3. Keine Cookies, kein Tracking, keine externen Inhalte</h2>
  <p>Wir setzen keine Cookies und speichern nichts im Browser. Es gibt keine Analyse- oder Werbedienste, keine Social-Media-Plugins und keine eingebetteten Inhalte Dritter. Schriften und Bilder werden von unserem eigenen Server geladen.</p>
  <p>Den Download-Button beschriften wir passend zu Ihrem Betriebssystem. Dafür wertet der Server den User-Agent aus, den Ihr Browser ohnehin mitschickt; gespeichert wird dabei nichts über die Logfiles hinaus.</p>

  <h2>4. Release-Informationen von GitHub</h2>
  <p>Versionsnummer, Download-Links, Sternzahl und Changelog ruft unser Server über die Programmierschnittstelle von GitHub (GitHub, Inc., 88 Colin P. Kelly Jr. Street, San Francisco, CA 94107, USA) ab und speichert sie zwischen. Diese Abfragen gehen vom Server aus; Daten über Sie als Besucher werden dabei nicht an GitHub übermittelt.</p>

  <h2>5. Links zu GitHub</h2>
  <p>Die Installer und der Quellcode liegen auf GitHub. Wenn Sie einen Download- oder GitHub-Link anklicken, verbindet sich Ihr Browser direkt mit GitHub. GitHub erhält dabei Ihre IP-Adresse und die üblichen Browserdaten und verarbeitet sie in eigener Verantwortung, auch in den USA. Details finden Sie im <a href="https://docs.github.com/de/site-policy/privacy-policies/github-general-privacy-statement">GitHub General Privacy Statement</a>.</p>

  <h2>6. Kontakt per E-Mail</h2>
  <p>Wenn Sie uns schreiben, verarbeiten wir Ihre Angaben, um Ihre Anfrage zu beantworten (Art. 6 Abs. 1 lit. b DSGVO bei vertragsbezogenen Anfragen, sonst lit. f). Die Daten werden gelöscht, sobald die Anfrage erledigt ist und keine gesetzlichen Aufbewahrungspflichten bestehen.</p>

  <h2>7. Ihre Rechte</h2>
  <p>Sie haben das Recht auf Auskunft (Art. 15 DSGVO), Berichtigung (Art. 16), Löschung (Art. 17), Einschränkung der Verarbeitung (Art. 18), Datenübertragbarkeit (Art. 20) und Widerspruch gegen Verarbeitungen auf Grundlage von Art. 6 Abs. 1 lit. f (Art. 21). Schreiben Sie dazu an <a href="mailto:<?= e($l['privacy_email']) ?>"><?= e($l['privacy_email']) ?></a>.</p>
  <p>Sie können sich außerdem bei einer Datenschutz-Aufsichtsbehörde beschweren (Art. 77 DSGVO), zum Beispiel bei der für uns zuständigen Landesbeauftragten für Datenschutz und Informationsfreiheit Nordrhein-Westfalen.</p>

  <p class="legal-meta">Stand: Oktober 2026</p>
  <p class="legal-links"><a href="/impressum">Impressum →</a></p>
</main>
<?php page_foot();
