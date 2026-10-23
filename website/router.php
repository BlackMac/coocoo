<?php
// Router for local development only (php -S does not read .htaccess); mirrors its rules:
//   php -S 127.0.0.1:8080 -t website/public website/router.php
declare(strict_types=1);

$root = __DIR__ . '/public';
$path = rawurldecode(parse_url($_SERVER['REQUEST_URI'], PHP_URL_PATH) ?: '/');

if (preg_match('#^/(inc|cache)(/|$)#', $path) || preg_match('#/\.(?!well-known/)#', $path)) {
    http_response_code(403);
    echo 'Forbidden';
    return true;
}
if (preg_match('#^/(changelog|impressum|datenschutz)/?$#', $path, $m)) {
    require $root . '/' . $m[1] . '.php';
    return true;
}
if ($path === '/') {
    require $root . '/index.php';
    return true;
}
if (is_file($root . $path)) {
    return false; // let the built-in server deliver the file
}
require $root . '/404.php';
return true;
