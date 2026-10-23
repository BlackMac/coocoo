<?php
declare(strict_types=1);

error_reporting(E_ALL);
ini_set('display_errors', '0');

function site(string $key): mixed
{
    static $config = null;
    $config ??= require __DIR__ . '/config.php';
    return $config[$key] ?? null;
}

function e(string $s): string
{
    return htmlspecialchars($s, ENT_QUOTES | ENT_SUBSTITUTE, 'UTF-8');
}

/** Root-relative asset URL with a modification-time query, so assets can be cached forever. */
function asset(string $path): string
{
    $file = dirname(__DIR__) . '/' . $path;
    return '/' . $path . (is_file($file) ? '?v=' . filemtime($file) : '');
}

function fmt_count(int $n): string
{
    if ($n >= 1000) {
        return rtrim(rtrim(number_format($n / 1000, 1, '.', ''), '0'), '.') . 'k';
    }
    return (string) $n;
}

function fmt_date(string $iso): string
{
    $ts = strtotime($iso);
    return $ts === false ? '' : date('M j, Y', $ts);
}

function fmt_size(int $bytes): string
{
    return $bytes > 0 ? number_format($bytes / 1048576, 1) . ' MB' : '';
}

require __DIR__ . '/github.php';
require __DIR__ . '/ua.php';
require __DIR__ . '/layout.php';
