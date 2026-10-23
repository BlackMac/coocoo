<?php
declare(strict_types=1);
require __DIR__ . '/inc/bootstrap.php';

$release = gh_latest();
$platform = detect_platform($_SERVER['HTTP_USER_AGENT'] ?? '');
$download = primary_download($platform, $release);
$installers = release_installers($release);
$stats = site('stats');
$version = $release['tag'];

// Pill text: the release title without the redundant "Coocoo vX.Y.Z".
$pillTitle = trim(str_ireplace(['coocoo', $version], '', $release['name']), " \t-–·:");
$pillTitle = $pillTitle !== '' ? $pillTitle : "What's new";

$primaryKind = PLATFORM_INSTALLER[$platform] ?? null;
$others = array_filter($installers, static fn ($k) => $k !== $primaryKind, ARRAY_FILTER_USE_KEY);

$jsonld = json_encode([
    '@context' => 'https://schema.org',
    '@type' => 'SoftwareApplication',
    'name' => 'Coocoo',
    'description' => 'Agentic development environment for omp: many coding agents, many machines, one calm overview.',
    'applicationCategory' => 'DeveloperApplication',
    'operatingSystem' => 'macOS, Windows, Linux',
    'softwareVersion' => ltrim($version, 'v'),
    'url' => site('site_url') . '/',
    'downloadUrl' => $release['html_url'],
    'license' => 'https://opensource.org/licenses/MIT',
    'offers' => ['@type' => 'Offer', 'price' => '0', 'priceCurrency' => 'EUR'],
    'publisher' => ['@type' => 'Organization', 'name' => 'YuzuHub UG (haftungsbeschränkt)', 'url' => site('legal')['website']],
], JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE | JSON_HEX_TAG);

page_head([
    'title' => 'Coocoo — Run ten agents. Answer only the ones that need you.',
    'description' => 'Coocoo runs omp coding agents in parallel on your laptop and your servers and puts everything that needs you in one queue. The daemon idles at 7 MB.',
    'path' => '/',
    'jsonld' => $jsonld,
    'vary_ua' => true,
    'body_class' => 'home',
]);

function download_button(array $download, array $release, string $extraClass = ''): void
{
    ?>
<a class="btn primary lg <?= e($extraClass) ?>" href="<?= e($download['href']) ?>" data-download<?= download_data_attrs($release) ?>>
  <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true"><path d="M8 2v8m0 0L5 7m3 3 3-3M3 13h10"/></svg><span data-download-label><?= e($download['label']) ?></span>
</a>
<?php
}
?>
<main id="main">
<header class="hero">
  <div class="hero-bg" aria-hidden="true">
    <div class="grid-bg"></div>
    <svg class="arcs" viewBox="0 0 1100 1100"><defs><linearGradient id="arc-g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#7a88ff" stop-opacity=".9"/><stop offset="1" stop-color="#4a57f0" stop-opacity="0"/></linearGradient></defs><circle class="a1" cx="550" cy="550" r="520"/><circle class="a2" cx="550" cy="550" r="300"/></svg>
  </div>
  <div class="wrap">
    <div class="hero-copy">
      <a class="pill" href="/changelog"><b><?= e($version) ?></b> <?= e($pillTitle) ?> <span aria-hidden="true">→</span></a>
      <h1><span class="soft">Run ten agents.</span><br>Answer only the ones that <span class="rot" data-rotate="need you.|are stuck.|are ready.|want a yes.">need you.</span></h1>
      <p class="lede">Coocoo runs <b>omp</b> coding agents in parallel on your laptop and your servers, and puts every question, approval and finished diff in <b>one queue</b>. The daemon idles at <b><?= (int) floor((float) $stats['daemon_rss_mb']) ?>&nbsp;MB</b>.</p>
      <div class="cta">
        <?php download_button($download, $release) ?>
        <a class="btn lg" href="#start" data-open-tab="tab-server"><span class="mono dim">$</span>&nbsp;coocoo serve</a>
      </div>
      <p class="cta-meta">
        <?= e($version) ?><?php if ($download['format'] !== ''): ?> · <?= e($download['format']) ?><?php endif ?>
        <?php if ($others !== []): ?> · <?= $download['installer'] ? 'also for' : 'for' ?>
          <?php $links = []; foreach ($others as $kind => $asset) { $links[] = '<a href="' . e($asset['url']) . '">' . e($kind === 'deb' ? '.deb' : INSTALLER_LABELS[$kind][0]) . '</a>'; } echo implode(', ', $links); ?>
        <?php endif ?>
        · MIT, free, open source
      </p>
      <p class="cta-meta" data-mobile-note<?= in_array($platform, ['ios', 'android'], true) ? '' : ' hidden' ?>>On your phone? Run <code>coocoo pair</code> on any machine with Coocoo and scan the QR code.</p>
    </div>

    <section class="stage" data-demo="hero" aria-label="Interactive demo of the Coocoo overview">
      <div class="try" data-try aria-hidden="true">Answer it — the agent is waiting</div>
      <div class="app">
        <div class="chrome" aria-hidden="true"><i></i><i></i><i></i><span>Coocoo — Overview</span></div>
        <div class="app-body">
          <aside class="side" aria-hidden="true">
            <div class="chips"><span class="chip on">All hosts</span><span class="chip"><span class="dot g"></span>studio</span><span class="chip"><span class="dot g"></span>gpu-box</span><span class="chip"><span class="dot g"></span>phone</span></div>
            <div class="nav-i on">▦ Overview <span class="badge" data-badge>0</span></div>
            <div class="nav-i">⌕ Jump to… <kbd>⌘K</kbd></div>
            <div class="sec-h">Projects</div>
            <div class="nav-i">▸ atlas-web</div>
            <div class="ag"><span class="dot b work" data-sdot="0"></span>Migrate billing to Stripe…</div>
            <div class="ag"><span class="dot b work" data-sdot="1"></span>Dark mode polish for inv…</div>
            <div class="nav-i">▸ coocoo</div>
            <div class="ag"><span class="dot g"></span>Fix WebSocket reconnect…</div>
            <div class="nav-i">▸ vision-train</div>
            <div class="ag"><span class="dot b work"></span>Port data loader to Web…</div>
          </aside>
          <div class="main">
            <div class="ov-h">
              <h2>Overview</h2>
              <div class="ov-st" aria-hidden="true">
                <span><span class="dot b"></span><b data-count-working>3</b><span class="lab"> working</span></span>
                <span><span class="dot o"></span><b data-count-need>0</b><span class="lab"> need you</span></span>
                <span><span class="dot g"></span><b data-count-ready>1</b><span class="lab"> ready</span></span>
              </div>
            </div>
            <div class="hosts" aria-hidden="true">
              <div class="host">
                <div class="hn">▭ studio <span class="dot g"></span><span class="n">2/3</span></div>
                <div class="bar">CPU<span class="tr"><i data-host="cpu:23:100:%"></i></span><span class="v">23%</span></div>
                <div class="bar">RAM<span class="tr"><i data-host="ram:42:64: GB"></i></span><span class="v">27.1 GB</span></div>
                <div class="bd">daemon <b><?= e($stats['daemon_rss_mb']) ?> MB</b> · omp <b>412 MB</b>×2</div>
              </div>
              <div class="host">
                <div class="hn">▤ gpu-box <span class="dot g"></span><span class="n">1/1</span></div>
                <div class="bar">CPU<span class="tr"><i data-host="cpu:60:100:%"></i></span><span class="v">60%</span></div>
                <div class="bar">RAM<span class="tr"><i data-host="ram:58:128: GB"></i></span><span class="v">73.9 GB</span></div>
                <div class="bd">daemon <b>8.1 MB</b> · omp <b>188 MB</b>×1</div>
              </div>
              <div class="host phone">
                <div class="hn">▯ phone <span class="dot g"></span><span class="n">viewer</span></div>
                <div class="bar">via<span class="tr"><i class="full"></i></span><span class="v">tailnet</span></div>
                <div class="bd">same UI in the browser · answers from the couch</div>
              </div>
            </div>
            <div class="q-h"><span>⌂ Needs you</span> <small data-queue-note>· nothing right now</small><span class="keys" aria-hidden="true"><kbd>J</kbd><kbd>K</kbd> move · <kbd>↵</kbd> open</span></div>
            <div class="queue">
              <div class="row" data-row="0">
                <span class="ic"><span class="dot b work" data-dot="0"></span></span>
                <div class="ti">Migrate billing to Stripe Checkout v3 <span class="chip"><span class="dot g"></span>studio</span></div>
                <div class="meta">2m</div>
                <div class="sub" data-sub="0"><span class="todo"><span class="prog"><i class="s2" data-prog="0"></i></span><span data-prog-t="0">2/5</span></span><span>Editing <code>billing/checkout.ts</code></span></div>
                <div class="ask" data-ask hidden>
                  <div class="lbl">QUESTION</div>
                  <p id="ask-q">Run the database migration <code>2026_10_23_add_checkout_sessions</code> against staging?</p>
                  <div class="opts" role="group" aria-labelledby="ask-q">
                    <button type="button" class="opt" data-answer="yes">Yes, run it <span class="rec">Recommended</span><kbd aria-hidden="true">1</kbd></button>
                    <button type="button" class="opt" data-answer="dry">Dry run first <kbd aria-hidden="true">2</kbd></button>
                    <button type="button" class="opt" data-answer="no">Not yet <kbd aria-hidden="true">3</kbd></button>
                  </div>
                </div>
              </div>
              <div class="row" data-row="1">
                <span class="ic"><span class="dot b work" data-dot="1"></span></span>
                <div class="ti">Dark mode polish for the invoices table <span class="chip"><span class="dot g"></span>studio</span></div>
                <div class="meta">6m</div>
                <div class="sub" data-sub="1"><span class="todo"><span class="prog"><i class="s3" data-prog="1"></i></span><span data-prog-t="1">3/5</span></span><span>Checking contrast in the browser</span></div>
              </div>
              <div class="row" data-row="2">
                <span class="ic"><span class="dot b work"></span></span>
                <div class="ti">Port data loader to WebDataset shards <span class="chip"><span class="dot g"></span>gpu-box</span></div>
                <div class="meta">11m</div>
                <div class="sub"><span class="todo"><span class="prog"><i class="s1" data-prog="2"></i></span><span data-prog-t="2">1/5</span></span><span>context 38% · $0.41</span></div>
              </div>
              <div class="row" data-row="3">
                <span class="ic"><span class="ok">✓</span></span>
                <div class="ti">Fix WebSocket reconnect backoff jitter <span class="chip"><span class="dot g"></span>studio</span></div>
                <div class="meta">18m</div>
                <div class="sub"><span class="diff">4 files <span class="p">+65</span> <span class="m">−7</span></span><span>Full-jitter exponential backoff, reset after 10 s stable.</span></div>
              </div>
            </div>
          </div>
        </div>
        <div class="toast" role="status" aria-live="polite" data-toast><span class="ok" aria-hidden="true">✓</span><span data-toast-text></span></div>
      </div>
      <p class="sr-only" aria-live="polite" data-announce></p>
    </section>
  </div>
</header>

<section class="proof wrap" aria-label="Measured resource use">
  <div class="nums reveal">
    <div class="num"><div class="v"><span data-count-to="<?= e($stats['daemon_rss_mb']) ?>"><?= e($stats['daemon_rss_mb']) ?></span><small>MB</small></div><div class="l">daemon memory, idle</div><div class="s">RSS, peered, one hibernated agent</div></div>
    <div class="num"><div class="v"><span data-count-to="0">0</span><small>CPU</small></div><div class="l">when nobody is watching</div><div class="s">0 ticks in 30 s, no polling</div></div>
    <div class="num"><div class="v"><span data-count-to="<?= e($stats['binary_mb']) ?>"><?= e($stats['binary_mb']) ?></span><small>MB</small></div><div class="l">one binary, web UI inside</div><div class="s">Rust daemon, no Node, no Electron</div></div>
    <div class="num"><div class="v"><span data-count-to="<?= e($stats['ui_kb']) ?>"><?= e($stats['ui_kb']) ?></span><small>KB</small></div><div class="l">initial UI bundle</div><div class="s">gzipped, Svelte 5</div></div>
  </div>
  <p class="note">Measured on <?= e($stats['measured']) ?>. <a href="<?= e($stats['method_url']) ?>">Method</a></p>
</section>

<section class="feat wrap" id="queue">
  <div class="split">
    <div class="txt reveal">
      <p class="eyebrow">01 · ONE QUEUE</p>
      <h2>Stop tab-hopping.<br><span class="dim">The agents come to you.</span></h2>
      <p class="big">Questions, approvals, errors and finished turns from every agent on every machine land in one needs-you queue. Triage it with J/K and answer inline.</p>
      <ul class="checks">
        <li><span><b>Structured questions,</b> not a terminal prompt you have to find. omp's ask dialogs become buttons.</span></li>
        <li><span><b>Diff stat, todo progress, context and cost</b> for every agent at a glance.</span></li>
        <li><span><b>Git state per agent:</b> branch, ahead/behind, conflicts, other agents in the same checkout.</span></li>
      </ul>
    </div>
    <div class="card glow reveal" data-demo="triage" aria-hidden="true">
      <div class="tri-keys"><kbd data-key="j">J</kbd><kbd data-key="k">K</kbd><span>triage</span></div>
      <div class="qrow sel" data-tri>
        <span class="ic"><span class="dot o pulse"></span></span>
        <div class="ti">Host stats panel with per-agent memory</div><div class="meta">4m</div>
        <div class="sub">How should CPU and memory history be shown?</div>
        <div class="opts"><span class="opt">Sparklines <span class="rec">Recommended</span></span><span class="opt">Stacked area</span><span class="opt">Numbers only</span></div>
      </div>
      <div class="qrow" data-tri>
        <span class="ic err">!</span>
        <div class="ti">Port data loader to WebDataset shards</div><div class="meta">11m</div>
        <div class="sub err">anthropic: 529 overloaded_error — retries exhausted (3/3)</div>
      </div>
      <div class="qrow" data-tri>
        <span class="ic ok">✓</span>
        <div class="ti">Dark mode polish for the invoices table</div><div class="meta">9m</div>
        <div class="sub"><span class="diff">3 files <span class="p">+10</span> <span class="m">−5</span></span> Ready for review</div>
      </div>
    </div>
  </div>
</section>

<section class="feat wrap" id="fleet">
  <div class="split rev">
    <div class="txt reveal">
      <p class="eyebrow">02 · EVERY MACHINE</p>
      <h2>Your laptop, your GPU box,<br><span class="dim">your phone. One window.</span></h2>
      <p class="big">The same daemon runs on desktops and headless servers. Link them over LAN, Tailscale or an SSH tunnel Coocoo supervises itself, and agents on one host can spawn and await agents on another.</p>
      <div class="term"><span class="c"># on the server</span><br><span class="pr">$</span> coocoo pair<br><span class="c">→ http://gpu-box:7421/#token=…  (QR code for your phone)</span><br><span class="pr">$</span> <span class="caret"></span></div>
    </div>
    <div class="card glow fleet reveal" data-demo="fleet" aria-hidden="true">
      <svg class="lines" data-lines></svg>
      <div class="node home n0"><div class="nh"><span class="dot g"></span>studio <span class="k">laptop · home</span></div><div class="mini"><i></i><i></i><i></i><i></i><i></i></div><div class="ram">daemon <?= e($stats['daemon_rss_mb']) ?> MB</div></div>
      <div class="node n1"><div class="nh"><span class="dot g"></span>gpu-box <span class="k">ssh tunnel</span></div><div class="mini"><i></i><i></i><i></i><i></i><i></i></div><div class="ram">daemon 8.1 MB</div></div>
      <div class="node n2"><div class="nh"><span class="dot g"></span>ci-runner <span class="k">tailnet</span></div><div class="mini"><i></i><i></i><i></i><i></i><i></i></div><div class="ram">daemon 7.9 MB</div></div>
    </div>
  </div>
</section>

<section class="feat wrap" id="browser">
  <div class="split">
    <div class="txt reveal">
      <p class="eyebrow">03 · AGENTS THAT SEE</p>
      <h2>One browser.<br><span class="dim">Agent drives, you can grab the wheel.</span></h2>
      <p class="big">Each project gets a Chromium owned by the daemon, on the host where the code runs. omp's browser tool drives it; you watch the same tab live and take over with mouse and keyboard, even from your phone.</p>
      <ul class="checks">
        <li><span><b>Design mode:</b> click an element, it becomes prompt context: screenshot, selector, computed styles, HTML.</span></li>
        <li><span><b>Starts on first use, stops when idle.</b> No browser eating RAM in the background.</span></li>
      </ul>
    </div>
    <div class="shot reveal" data-demo="picker">
      <img src="<?= e(asset('assets/img/agent-browser.webp')) ?>" srcset="<?= e(asset('assets/img/agent-browser-720.webp')) ?> 720w, <?= e(asset('assets/img/agent-browser.webp')) ?> 1280w" sizes="(max-width: 900px) 100vw, 640px" width="1280" height="800" loading="lazy" decoding="async" alt="A Coocoo agent driving the project's browser, with the live view on the right">
      <div class="pk pk0 on" aria-hidden="true"><span>h1 · Demo Counter</span></div>
      <div class="pk pk1" aria-hidden="true"><span>button#inc · design mode</span></div>
      <div class="pk pk2" aria-hidden="true"><span>button · Reset</span></div>
      <div class="cur-track c0" aria-hidden="true"><svg class="cursor" viewBox="0 0 24 24"><path d="M3 2l7 19 2.5-7.5L20 11z" fill="#fff" stroke="#000" stroke-width="1.2"/></svg></div>
    </div>
  </div>
</section>

<section class="feat wrap" id="ram">
  <div class="split rev">
    <div class="txt reveal">
      <p class="eyebrow">04 · STAYS OUT OF YOUR RAM</p>
      <h2>The memory goes to<br><span class="dim">the agents. Not to us.</span></h2>
      <p class="big">A Rust daemon instead of Node and Electron, a Tauri shell on the system webview. Idle agents hibernate after <?= (int) $stats['hibernate_min'] ?> minutes and resume their omp session on the next prompt.</p>
    </div>
    <div class="card glow reveal" data-demo="ram">
      <div class="ramviz">
        <div class="rv"><span class="lab">Coocoo daemon</span><span class="tr"><i class="daemon"></i></span><span class="val"><?= e($stats['daemon_rss_mb']) ?> MB</span></div>
        <div class="rv"><span class="lab">omp agent, busy</span><span class="tr"><i class="agent"></i></span><span class="val"><?= e($stats['agent_mb']) ?> MB</span></div>
        <div class="rv"><span class="lab">Chromium tab</span><span class="tr"><i class="chromium"></i></span><span class="val"><?= e($stats['chromium_mb']) ?> MB</span></div>
        <div class="rv"><span class="lab">hibernated agent</span><span class="tr"><i class="hib"></i></span><span class="val">0 MB</span></div>
      </div>
      <div class="timeline">
        <p class="tl-h">One agent, one afternoon</p>
        <div class="tl"><span class="tl-l">status</span><span class="segs"><i class="s-work w30"></i><i class="s-wait w8"></i><i class="s-work w14"></i><i class="s-idle w18"></i><i class="s-hib w30"></i></span></div>
        <div class="tl"><span class="tl-l">memory</span><span class="segs"><i class="s-mem w70"></i><i class="s-zero w30"></i></span></div>
        <p class="tl-n">Idle for <?= (int) $stats['hibernate_min'] ?> min → the process exits, the session stays → 0 MB until your next prompt.</p>
      </div>
    </div>
  </div>
</section>

<section class="feat wrap" id="kanban">
  <div class="split">
    <div class="txt reveal">
      <p class="eyebrow">05 · QUEUE THE WORK</p>
      <h2>A board per project.<br><span class="dim">Agents finish their own cards.</span></h2>
      <p class="big">Ask for a refinement and the agent researches, asks you the rest, and attaches a plan with UI mocks. Hand the card over and it commits when done. Importers fill the inbox from RSS, any command, or a script.</p>
    </div>
    <div class="card glow reveal" data-demo="kanban" aria-hidden="true">
      <div class="kb" data-board>
        <div class="col"><h3>Inbox <span>2</span></h3><div class="kc">Homepage<div class="who"><span class="dot"></span>refined · 2 mocks</div></div><div class="kc">Changelog page<div class="who"><span class="dot"></span>inbox · from RSS</div></div></div>
        <div class="col"><h3>Todo <span>2</span></h3><div class="kc">Dark mode for diff view<div class="who"><span class="dot"></span>todo</div></div><div class="kc">SSH peer docs<div class="who"><span class="dot"></span>todo</div></div></div>
        <div class="col"><h3>Doing <span>1</span></h3><div class="kc">Host stats history<div class="who"><span class="dot b work"></span>agent · studio</div></div></div>
        <div class="col"><h3>Done <span>1</span></h3><div class="kc">Fix reconnect jitter<div class="who"><span class="dot g"></span>committed a41f2c9</div></div></div>
      </div>
    </div>
  </div>
</section>

<section class="feat wrap" id="omp">
  <div class="center reveal">
    <p class="eyebrow">06 · BUILT FOR OMP</p>
    <h2>No lowest common denominator.</h2>
    <p class="big">Coocoo speaks <a href="https://github.com/can1357/oh-my-pi">omp</a>'s RPC protocol directly. That is why it can show you things other wrappers can't.</p>
  </div>
  <div class="omp">
    <div class="card glow reveal"><div class="ic2" aria-hidden="true">?</div><h3>Real dialogs</h3><p>ask, confirm, select and input become answerable cards on any client.</p></div>
    <div class="card glow reveal"><div class="ic2" aria-hidden="true">⇄</div><h3>Cross-host orchestration</h3><p>Spawn, message and wait for agents on any host. Injected as host tools, no MCP server.</p></div>
    <div class="card glow reveal"><div class="ic2" aria-hidden="true">%</div><h3>Quota where you type</h3><p>The composer shows your provider's 5-hour and weekly windows, the way the provider meters them.</p></div>
  </div>
</section>

<section class="start wrap" id="start">
  <div class="reveal">
    <p class="eyebrow">GET STARTED</p>
    <h2>Two minutes to your first fleet.</h2>
    <p class="big">You need <a href="https://github.com/can1357/oh-my-pi">omp</a> installed and logged in, plus git. Chromium is optional, for browser features.</p>
    <div class="tabs" role="tablist" aria-label="Setup">
      <button type="button" role="tab" id="tab-desktop" aria-controls="panel-desktop" aria-selected="true">Desktop</button>
      <button type="button" role="tab" id="tab-server" aria-controls="panel-server" aria-selected="false" tabindex="-1">Server</button>
      <button type="button" role="tab" id="tab-phone" aria-controls="panel-phone" aria-selected="false" tabindex="-1">Phone</button>
    </div>
    <div class="term panel" role="tabpanel" id="panel-desktop" aria-labelledby="tab-desktop">
      <div><span class="c"># download the installer for your OS and open it</span></div>
      <div><span class="c"># macOS: first launch with right-click → Open (ad-hoc signed, not notarized)</span></div>
      <div><span class="c"># closing the window hides it to the tray, agents keep running</span></div>
    </div>
    <div class="term panel" role="tabpanel" id="panel-server" aria-labelledby="tab-server" hidden>
      <button type="button" class="copy" data-copy>Copy</button>
      <div><span class="c"># easiest: desktop app → Hosts &amp; Settings → Add host → SSH → user@server, tick “install”</span></div>
      <div><span class="c"># or build it on the server:</span></div>
      <div><span class="pr">$</span> <span data-cmd>git clone <?= e(repo_url()) ?> &amp;&amp; cd coocoo</span></div>
      <div><span class="pr">$</span> <span data-cmd>cargo build --release -p coocoo-cli</span></div>
      <div><span class="pr">$</span> <span data-cmd>./target/release/coocoo serve --listen 0.0.0.0:7421</span> <span class="c"># LAN / Tailscale</span></div>
      <div><span class="pr">$</span> <span data-cmd>./target/release/coocoo pair</span> <span class="c"># connect link + QR code</span></div>
    </div>
    <div class="term panel" role="tabpanel" id="panel-phone" aria-labelledby="tab-phone" hidden>
      <button type="button" class="copy" data-copy>Copy</button>
      <div><span class="pr">$</span> <span data-cmd>coocoo pair</span> <span class="c"># on any machine running Coocoo</span></div>
      <div><span class="c"># scan the QR code: same UI as the desktop, on your LAN or tailnet</span></div>
    </div>
  </div>

  <div class="final reveal" id="download">
    <svg class="final-logo" aria-hidden="true"><use href="#logo"/></svg>
    <h2>Let them work.<br>You decide.</h2>
    <div class="cta">
      <?php download_button($download, $release) ?>
      <a class="btn lg" href="<?= e(repo_url()) ?>">★ Star on GitHub</a>
    </div>
    <?php if ($installers !== []): ?>
    <ul class="os">
      <?php foreach ($installers as $kind => $asset): ?>
      <li><a href="<?= e($asset['url']) ?>"><?= e(INSTALLER_LABELS[$kind][0] . ' ' . INSTALLER_LABELS[$kind][1]) ?></a><?php if ($asset['size'] > 0): ?> <span><?= e(fmt_size($asset['size'])) ?></span><?php endif ?></li>
      <?php endforeach ?>
    </ul>
    <?php endif ?>
    <p class="os-note"><?= e($version) ?> · <a href="<?= e($release['html_url']) ?>">release notes</a> · macOS: first launch with right-click → Open</p>
  </div>
</section>
</main>
<?php page_foot();
