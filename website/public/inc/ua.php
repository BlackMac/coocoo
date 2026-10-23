<?php
// Visitor platform detection and release-asset mapping for the download buttons.
declare(strict_types=1);

/** @return 'mac'|'windows'|'linux'|'ios'|'android'|'other' */
function detect_platform(string $ua): string
{
    return match (true) {
        (bool) preg_match('/iPhone|iPad|iPod/i', $ua) => 'ios',
        (bool) preg_match('/Android/i', $ua) => 'android',
        (bool) preg_match('/Windows NT|Win64|Win32/i', $ua) => 'windows',
        (bool) preg_match('/Macintosh|Mac OS X/i', $ua) => 'mac',
        (bool) preg_match('/CrOS/i', $ua) => 'other',
        (bool) preg_match('/Linux|X11/i', $ua) => 'linux',
        default => 'other',
    };
}

/**
 * Installers of a release by kind, matched by file suffix.
 *
 * @return array<'dmg'|'exe'|'appimage'|'deb', array{name:string,url:string,size:int}>
 */
function release_installers(array $release): array
{
    $patterns = [
        'dmg' => '/\.dmg$/i',
        'exe' => '/-setup\.exe$/i',
        'appimage' => '/\.AppImage$/i',
        'deb' => '/\.deb$/i',
    ];
    $found = [];
    foreach ($release['assets'] ?? [] as $asset) {
        foreach ($patterns as $kind => $re) {
            if (!isset($found[$kind]) && preg_match($re, $asset['name'])) {
                $found[$kind] = $asset;
            }
        }
    }
    // Stable order (macOS, Windows, AppImage, .deb) regardless of the asset order on GitHub.
    return array_intersect_key(array_replace(array_fill_keys(array_keys($patterns), null), $found), $found);
}

const INSTALLER_LABELS = [
    'dmg' => ['macOS', 'universal .dmg'],
    'exe' => ['Windows', 'x64 installer'],
    'appimage' => ['Linux', 'AppImage'],
    'deb' => ['Linux', '.deb'],
];

const PLATFORM_INSTALLER = ['mac' => 'dmg', 'windows' => 'exe', 'linux' => 'appimage'];

/**
 * The primary download for a platform: label, href, format and whether it is an installer.
 *
 * @return array{label:string,href:string,format:string,installer:bool}
 */
function primary_download(string $platform, array $release): array
{
    $installers = release_installers($release);
    $kind = PLATFORM_INSTALLER[$platform] ?? null;
    if ($kind !== null && isset($installers[$kind])) {
        return [
            'label' => 'Download for ' . INSTALLER_LABELS[$kind][0],
            'href' => $installers[$kind]['url'],
            'format' => INSTALLER_LABELS[$kind][1],
            'installer' => true,
        ];
    }
    if ($platform === 'ios' || $platform === 'android') {
        return ['label' => 'Get it on your computer', 'href' => '#start', 'format' => '', 'installer' => false];
    }
    return ['label' => 'Download ' . $release['tag'], 'href' => $release['html_url'], 'format' => '', 'installer' => false];
}

/**
 * Attributes that let the client script re-label the button (e.g. iPadOS reports a Mac UA).
 */
function download_data_attrs(array $release): string
{
    $attrs = ['data-release' => $release['html_url'], 'data-tag' => $release['tag']];
    foreach (release_installers($release) as $kind => $asset) {
        $attrs['data-' . $kind] = $asset['url'];
    }
    $out = '';
    foreach ($attrs as $k => $v) {
        $out .= ' ' . $k . '="' . e($v) . '"';
    }
    return $out;
}
