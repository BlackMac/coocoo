<script lang="ts">
  import { fleet } from '../lib/fleet.svelte';
  import type { Project } from '../lib/protocol';

  /** A project's persona avatar; the persona's initial on its color until the image is loaded or without one. */
  let { project, size = 14 }: { project: Project; size?: number } = $props();

  const persona = $derived(project.persona);
  const src = $derived(fleet.personaAvatar(project));
</script>

{#if src}
  <img class="avatar" {src} alt="" style:--size="{size}px" />
{:else if persona}
  <span class="avatar initial" style:--size="{size}px" style:background={persona.color ?? 'var(--accent)'} aria-hidden="true">{[...persona.name][0]?.toUpperCase()}</span>
{/if}

<style>
  .avatar {
    width: var(--size);
    height: var(--size);
    flex: none;
    border-radius: 50%;
    object-fit: cover;
  }
  .initial {
    display: grid;
    place-items: center;
    color: #fff;
    font-size: calc(var(--size) * 0.6);
    font-weight: 600;
    line-height: 1;
  }
</style>
