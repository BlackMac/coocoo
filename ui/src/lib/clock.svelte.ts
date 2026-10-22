// One shared, coarse clock for relative timestamps (a single timer for the whole UI, paused
// while the page is hidden).

class Clock {
  value = $state(Date.now());

  constructor() {
    let timer: number | undefined;
    const start = () => {
      clearInterval(timer);
      this.value = Date.now();
      timer = setInterval(() => (this.value = Date.now()), 10_000);
    };
    document.addEventListener('visibilitychange', () => {
      if (document.visibilityState === 'visible') start();
      else clearInterval(timer);
    });
    start();
  }
}

export const now = new Clock();
