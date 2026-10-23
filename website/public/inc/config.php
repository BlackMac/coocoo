<?php
// Site configuration. Values here are read by every page; nothing secret is committed.
declare(strict_types=1);

return [
    'site_url' => 'https://coocoo.dev',
    'repo' => 'BlackMac/coocoo',

    // Optional GitHub token (raises the API limit from 60 to 5000 requests/hour).
    // Set COOCOO_GITHUB_TOKEN in the environment, e.g. `SetEnv COOCOO_GITHUB_TOKEN …` in a
    // server-side .htaccess that is not in git. Never commit a token here.
    'github_token' => getenv('COOCOO_GITHUB_TOKEN') ?: '',

    // GitHub API caching (seconds). Failed requests pause further calls for `github_retry`.
    'github_ttl' => 3600,
    'github_retry' => 300,
    'github_timeout' => 2,

    // Hide the star count until the repo has at least this many stars (0 = always show).
    'stars_min' => 0,

    // Used when GitHub is unreachable and nothing is cached yet. Update on each release.
    'fallback_release' => [
        'tag' => 'v0.3.0',
        'name' => 'Coocoo v0.3.0',
        'published_at' => '2026-10-23T21:56:38Z',
        'html_url' => 'https://github.com/BlackMac/coocoo/releases/tag/v0.3.0',
        'body_html' => '',
        'assets' => [
            ['name' => 'Coocoo_0.3.0_universal.dmg', 'url' => 'https://github.com/BlackMac/coocoo/releases/download/v0.3.0/Coocoo_0.3.0_universal.dmg', 'size' => 0],
            ['name' => 'Coocoo_0.3.0_x64-setup.exe', 'url' => 'https://github.com/BlackMac/coocoo/releases/download/v0.3.0/Coocoo_0.3.0_x64-setup.exe', 'size' => 0],
            ['name' => 'Coocoo_0.3.0_amd64.AppImage', 'url' => 'https://github.com/BlackMac/coocoo/releases/download/v0.3.0/Coocoo_0.3.0_amd64.AppImage', 'size' => 0],
            ['name' => 'Coocoo_0.3.0_amd64.deb', 'url' => 'https://github.com/BlackMac/coocoo/releases/download/v0.3.0/Coocoo_0.3.0_amd64.deb', 'size' => 0],
        ],
    ],

    // Measured numbers shown on the homepage. Keep in sync with README.md "Resource use".
    'stats' => [
        'daemon_rss_mb' => '7.6',
        'binary_mb' => '5.4',
        'ui_kb' => '91',
        'measured' => 'Linux x86_64, release builds, 2026-10-22',
        'method_url' => 'https://github.com/BlackMac/coocoo#resource-use',
        'agent_mb' => '150–210',
        'chromium_mb' => '~350',
        'hibernate_min' => 15,
    ],

    // Impressum / Datenschutz.
    'legal' => [
        'company' => 'YuzuHub UG (haftungsbeschränkt)',
        'street' => 'Sedanstr. 3',
        'city' => '40217 Düsseldorf',
        'country' => 'Deutschland',
        'phone' => '+49 (0) 211 63556455',
        'email' => 'stefan@yuzuhub.com',
        'privacy_email' => 'privacy@yuzuhub.com',
        'website' => 'https://yuzuhub.com/',
        'managing_directors' => 'Verena Wanner, Stefan Lange-Hegermann',
        'register' => 'Amtsgericht Düsseldorf, HRB 108734',
        'vat_id' => 'DE455375588',
    ],
];
