// Hash routing: `#/`, `#/p/<host>/<project>`, `#/a/<host>/<agent>`, `#/settings`.

export type Route =
  | { name: 'overview' }
  | { name: 'project'; host: string; id: string }
  | { name: 'agent'; host: string; id: string }
  | { name: 'settings' };

function parse(hash: string): Route {
  const parts = hash.replace(/^#\/?/, '').split('/').filter(Boolean).map(decodeURIComponent);
  if (parts[0] === 'p' && parts.length === 3) return { name: 'project', host: parts[1], id: parts[2] };
  if (parts[0] === 'a' && parts.length === 3) return { name: 'agent', host: parts[1], id: parts[2] };
  if (parts[0] === 'settings') return { name: 'settings' };
  return { name: 'overview' };
}

export function href(route: Route): string {
  switch (route.name) {
    case 'overview':
      return '#/';
    case 'settings':
      return '#/settings';
    case 'project':
      return `#/p/${encodeURIComponent(route.host)}/${encodeURIComponent(route.id)}`;
    case 'agent':
      return `#/a/${encodeURIComponent(route.host)}/${encodeURIComponent(route.id)}`;
  }
}

class Router {
  route = $state<Route>(parse(location.hash));

  constructor() {
    window.addEventListener('hashchange', () => {
      this.route = parse(location.hash);
    });
  }

  go(route: Route): void {
    const h = href(route);
    if (location.hash !== h) location.hash = h;
  }
}

export const router = new Router();
