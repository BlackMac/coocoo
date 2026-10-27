// Hash routing: `#/`, `#/stats`, `#/workflows`, `#/p/<host>/<project>`, `#/a/<host>/<agent>[?at=<ms>]`,
// `#/settings`, `#/vibe`.

export type Route =
  | { name: 'overview' }
  | { name: 'stats' }
  | { name: 'workflows' }
  | { name: 'project'; host: string; id: string }
  /** `at`: scroll the transcript to this moment (Unix ms), e.g. from a workflow example. */
  | { name: 'agent'; host: string; id: string; at?: number }
  | { name: 'settings' }
  /** Full-screen controller view: every agent's state on one display, answers and prompts by voice. */
  | { name: 'vibe' };

function parse(hash: string): Route {
  const [path, query = ''] = hash.replace(/^#\/?/, '').split('?');
  const parts = path.split('/').filter(Boolean).map(decodeURIComponent);
  if (parts[0] === 'p' && parts.length === 3) return { name: 'project', host: parts[1], id: parts[2] };
  if (parts[0] === 'a' && parts.length === 3) {
    const at = Number(new URLSearchParams(query).get('at'));
    return { name: 'agent', host: parts[1], id: parts[2], ...(at > 0 ? { at } : {}) };
  }
  if (parts[0] === 'settings') return { name: 'settings' };
  if (parts[0] === 'stats') return { name: 'stats' };
  if (parts[0] === 'workflows') return { name: 'workflows' };
  if (parts[0] === 'vibe') return { name: 'vibe' };
  return { name: 'overview' };
}

export function href(route: Route): string {
  switch (route.name) {
    case 'overview':
      return '#/';
    case 'stats':
      return '#/stats';
    case 'workflows':
      return '#/workflows';
    case 'settings':
      return '#/settings';
    case 'vibe':
      return '#/vibe';
    case 'project':
      return `#/p/${encodeURIComponent(route.host)}/${encodeURIComponent(route.id)}`;
    case 'agent':
      return `#/a/${encodeURIComponent(route.host)}/${encodeURIComponent(route.id)}${route.at ? `?at=${route.at}` : ''}`;
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
