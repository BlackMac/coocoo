<?php
// GitHub API access with a JSON file cache. Pages never wait on GitHub for longer than one
// request timeout: a failed request switches to stale cache / config fallback for `github_retry`
// seconds, and no further requests are made during that time.
declare(strict_types=1);

const GH_API = 'https://api.github.com';

function gh_cache_dir(): string
{
    return dirname(__DIR__) . '/cache';
}

function gh_cache_key(string $key): string
{
    return preg_replace('/[^A-Za-z0-9._-]/', '_', $key);
}

/** @return array{t:int,v:mixed}|null */
function gh_cache_read(string $key): ?array
{
    $file = gh_cache_dir() . '/' . gh_cache_key($key) . '.json';
    if (!is_file($file)) {
        return null;
    }
    $raw = @file_get_contents($file);
    $data = $raw === false ? null : json_decode($raw, true);
    return is_array($data) && isset($data['t']) && array_key_exists('v', $data) ? $data : null;
}

function gh_cache_write(string $key, mixed $value): void
{
    $dir = gh_cache_dir();
    if (!is_dir($dir) && !@mkdir($dir, 0755, true)) {
        return;
    }
    $tmp = @tempnam($dir, 'tmp');
    if ($tmp === false) {
        return;
    }
    $json = json_encode(['t' => time(), 'v' => $value], JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE);
    if ($json === false || @file_put_contents($tmp, $json) === false) {
        @unlink($tmp);
        return;
    }
    @chmod($tmp, 0644);
    if (!@rename($tmp, $dir . '/' . gh_cache_key($key) . '.json')) {
        @unlink($tmp);
    }
}

function gh_offline(): bool
{
    $mark = gh_cache_read('backoff');
    return $mark !== null && time() - $mark['t'] < (int) site('github_retry');
}

function gh_mark_offline(): void
{
    gh_cache_write('backoff', true);
}

/** @return list<string> */
function gh_headers(bool $html): array
{
    $headers = [
        'Accept: ' . ($html ? 'application/vnd.github.html+json' : 'application/vnd.github+json'),
        'User-Agent: coocoo.dev',
        'X-GitHub-Api-Version: 2022-11-28',
    ];
    $token = (string) site('github_token');
    if ($token !== '') {
        $headers[] = 'Authorization: Bearer ' . $token;
    }
    return $headers;
}

/**
 * Fetches several API paths in parallel. A path maps to its decoded JSON, or null on any failure.
 * Any failure marks GitHub as offline so the rest of this and the following requests skip it.
 *
 * @param list<string> $paths
 * @return array<string, array|null>
 */
function gh_fetch(array $paths, bool $html = false): array
{
    $out = array_fill_keys($paths, null);
    if ($paths === [] || gh_offline()) {
        return $out;
    }
    $timeout = (int) site('github_timeout');
    $headers = gh_headers($html);

    if (function_exists('curl_multi_init')) {
        $multi = curl_multi_init();
        $handles = [];
        foreach ($paths as $path) {
            $ch = curl_init(GH_API . $path);
            curl_setopt_array($ch, [
                CURLOPT_RETURNTRANSFER => true,
                CURLOPT_HTTPHEADER => $headers,
                CURLOPT_CONNECTTIMEOUT => $timeout,
                CURLOPT_TIMEOUT => $timeout,
                CURLOPT_FOLLOWLOCATION => true,
                CURLOPT_ENCODING => '',
            ]);
            curl_multi_add_handle($multi, $ch);
            $handles[$path] = $ch;
        }
        do {
            $status = curl_multi_exec($multi, $running);
            if ($running) {
                curl_multi_select($multi, 0.2);
            }
        } while ($running && $status === CURLM_OK);
        foreach ($handles as $path => $ch) {
            $code = (int) curl_getinfo($ch, CURLINFO_RESPONSE_CODE);
            $body = curl_multi_getcontent($ch);
            if ($code >= 200 && $code < 300 && is_string($body)) {
                $json = json_decode($body, true);
                $out[$path] = is_array($json) ? $json : null;
            }
            curl_multi_remove_handle($multi, $ch);
        }
        curl_multi_close($multi);
    } elseif (filter_var(ini_get('allow_url_fopen'), FILTER_VALIDATE_BOOLEAN)) {
        // Sequential fallback for PHP builds without cURL (e.g. a bare local php-cli).
        foreach ($paths as $path) {
            $ctx = stream_context_create(['http' => [
                'method' => 'GET',
                'header' => implode("\r\n", $headers),
                'timeout' => $timeout,
                'ignore_errors' => true,
            ]]);
            $body = @file_get_contents(GH_API . $path, false, $ctx);
            $statusLine = function_exists('http_get_last_response_headers')
                ? (http_get_last_response_headers()[0] ?? '')
                : ($http_response_header[0] ?? '');
            if ($body !== false && preg_match('#^HTTP/\S+ 2\d\d#', $statusLine)) {
                $json = json_decode($body, true);
                $out[$path] = is_array($json) ? $json : null;
            }
            if ($out[$path] === null) {
                break;
            }
        }
    }

    if (in_array(null, $out, true)) {
        gh_mark_offline();
    }
    return $out;
}

/**
 * Returns the cached value for $key, refreshing it with $fetch when older than $ttl
 * ($ttl 0 = never expires). A failed refresh keeps the stale value.
 */
function gh_cached(string $key, int $ttl, callable $fetch): mixed
{
    $cached = gh_cache_read($key);
    if ($cached !== null && ($ttl === 0 || time() - $cached['t'] < $ttl)) {
        return $cached['v'];
    }
    $value = gh_offline() ? null : $fetch();
    if ($value === null) {
        return $cached['v'] ?? null;
    }
    gh_cache_write($key, $value);
    return $value;
}

function gh_stars(): ?int
{
    $repo = site('repo');
    $data = gh_cached('repo', (int) site('github_ttl'), static function () use ($repo) {
        $r = gh_fetch(["/repos/$repo"])["/repos/$repo"];
        return isset($r['stargazers_count']) ? ['stars' => (int) $r['stargazers_count']] : null;
    });
    return isset($data['stars']) ? (int) $data['stars'] : null;
}

/**
 * Published releases, newest first.
 *
 * @return list<array{tag:string,name:string,published_at:string,html_url:string,body_html:string,prerelease:bool,assets:list<array{name:string,url:string,size:int}>}>
 */
function gh_releases(): array
{
    $repo = site('repo');
    $list = gh_cached('releases', (int) site('github_ttl'), static function () use ($repo) {
        $path = "/repos/$repo/releases?per_page=30";
        $raw = gh_fetch([$path], true)[$path];
        if (!is_array($raw) || !array_is_list($raw)) {
            return null;
        }
        $releases = [];
        foreach ($raw as $r) {
            if (!empty($r['draft']) || !isset($r['tag_name'])) {
                continue;
            }
            $assets = [];
            foreach ($r['assets'] ?? [] as $a) {
                $assets[] = ['name' => (string) $a['name'], 'url' => (string) $a['browser_download_url'], 'size' => (int) ($a['size'] ?? 0)];
            }
            $releases[] = [
                'tag' => (string) $r['tag_name'],
                'name' => (string) ($r['name'] ?: $r['tag_name']),
                'published_at' => (string) ($r['published_at'] ?? ''),
                'html_url' => (string) $r['html_url'],
                'body_html' => (string) ($r['body_html'] ?? ''),
                'prerelease' => !empty($r['prerelease']),
                'assets' => $assets,
            ];
        }
        return $releases;
    });
    if (is_array($list) && $list !== []) {
        return $list;
    }
    return [site('fallback_release') + ['prerelease' => false, 'fallback' => true]];
}

/** The newest non-prerelease (or the config fallback). */
function gh_latest(): array
{
    foreach (gh_releases() as $r) {
        if (!$r['prerelease']) {
            return $r;
        }
    }
    return site('fallback_release') + ['prerelease' => false, 'fallback' => true];
}

/**
 * Commit subjects between consecutive releases, keyed by the newer tag. Compare results never
 * change, so they are cached permanently; at most 10 missing ones are fetched per request.
 *
 * @param list<array{tag:string}> $releases newest first
 * @return array<string, array{commits:list<array{sha:string,subject:string,url:string}>,total:int,url:string}>
 */
function gh_compares(array $releases): array
{
    $repo = site('repo');
    $pairs = [];
    for ($i = 0; $i < count($releases) - 1; $i++) {
        $pairs[$releases[$i]['tag']] = $releases[$i + 1]['tag'];
    }
    $result = [];
    $missing = [];
    foreach ($pairs as $tag => $prev) {
        $hit = gh_cache_read("compare-$prev...$tag");
        if ($hit !== null) {
            $result[$tag] = $hit['v'];
        } elseif (count($missing) < 10) {
            $missing["/repos/$repo/compare/" . rawurlencode($prev) . '...' . rawurlencode($tag)] = [$prev, $tag];
        }
    }
    if ($missing !== []) {
        foreach (gh_fetch(array_keys($missing)) as $path => $data) {
            if (!isset($data['commits']) || !is_array($data['commits'])) {
                continue;
            }
            [$prev, $tag] = $missing[$path];
            $commits = [];
            foreach (array_reverse($data['commits']) as $c) {
                if (count($c['parents'] ?? []) > 1) {
                    continue; // merge commits
                }
                $commits[] = [
                    'sha' => substr((string) $c['sha'], 0, 7),
                    'subject' => strtok((string) ($c['commit']['message'] ?? ''), "\n") ?: '',
                    'url' => (string) $c['html_url'],
                ];
            }
            $value = ['commits' => $commits, 'total' => (int) ($data['total_commits'] ?? count($commits)), 'url' => (string) ($data['html_url'] ?? '')];
            gh_cache_write("compare-$prev...$tag", $value);
            $result[$tag] = $value;
        }
    }
    return $result;
}
