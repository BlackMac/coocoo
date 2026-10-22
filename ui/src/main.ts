import './app.css';
import { mount } from 'svelte';
import App from './App.svelte';

// iOS zooms into every input with a font smaller than 16px when it gets focus. `maximum-scale=1`
// stops that; iOS still allows pinch-zoom with it, but Android would not, so only on iOS
// (iPadOS reports itself as a Mac with touch).
if (/iPad|iPhone|iPod/.test(navigator.userAgent) || (navigator.platform === 'MacIntel' && navigator.maxTouchPoints > 1)) {
  document.querySelector('meta[name="viewport"]')?.setAttribute('content', 'width=device-width, initial-scale=1, maximum-scale=1, viewport-fit=cover');
}

mount(App, { target: document.getElementById('app')! });
