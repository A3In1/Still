<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { tick } from 'svelte';
  import { formatBytes, formatDayDate, formatTime, formatDuration } from '../utils';
  import type { Photo } from '../types';
  import { cameraName, cameraRows, videoRows, imageRows, locationRows, descriptionRows, timeZone, type MediaDetails } from '../mediaDetails';
  import 'leaflet/dist/leaflet.css';

  export let photo: Photo;
  export let onClose: () => void;
  export let onUpdated: (updated: Photo) => void;

  let isEditingDate = false;
  let editDateValue = '';
  let isEditingFileName = false;
  let editFileNameValue = '';
  let isEditingLocation = false;
  let locationSearchQuery = '';

  let mapContainer: HTMLElement;
  let leafletMap: any = null;
  let leafletMarker: any = null;
  let editLat = 0;
  let editLng = 0;

  $: if (photo) {
    isEditingDate = false;
    isEditingFileName = false;
    isEditingLocation = false;
  }

  // Camera/codec details are read from the file itself; anything unknown is simply not shown.
  let details: MediaDetails = {};
  let detailsPath = '';
  $: if (photo && photo.path !== detailsPath) void loadDetails(photo.path);
  async function loadDetails(path: string) {
    detailsPath = path;
    details = {};
    try {
      const result = await invoke<MediaDetails>('get_media_details', { path });
      if (path === detailsPath) details = result ?? {};
    } catch (e) {
      console.error('Failed to read media details:', e);
    }
  }
  $: isVideo = photo.media_type === 'video';
  $: camera = cameraName(details);
  $: cameraList = cameraRows(details);
  $: streamList = isVideo ? videoRows(details) : imageRows(details);
  $: placeList = locationRows(details);
  $: aboutList = descriptionRows(details);
  $: zone = timeZone(details);
  $: pairedFile = typeof details.paired_file === 'string' ? details.paired_file : null;

  async function saveDate() {
    try {
      const dbDate = editDateValue.replace('T', ' ') + (editDateValue.length === 16 ? ':00' : '');
      await invoke('update_image_metadata', {
        path: photo.path,
        newDate: editDateValue,
        newLat: null,
        newLng: null
      });
      photo = { ...photo, mtime: Math.floor(new Date(dbDate).getTime() / 1000) };
      onUpdated(photo);
      isEditingDate = false;
    } catch (e) {
      console.error('Failed to save date:', e);
    }
  }

  async function saveFileName() {
    if (!editFileNameValue.trim()) return;
    try {
      const newPath = await invoke<string>('rename_image', {
        oldPath: photo.path,
        newName: editFileNameValue
      });
      photo = { ...photo, path: newPath };
      onUpdated(photo);
      isEditingFileName = false;
    } catch (e) {
      console.error('Failed to rename:', e);
    }
  }

  async function startEditLocation() {
    isEditingLocation = true;
    editLat = photo.lat || 0;
    editLng = photo.lng || 0;
    await tick();
    if (!leafletMap && mapContainer) {
      const L = await import('leaflet');
      delete (L.Icon.Default.prototype as any)._getIconUrl;
      L.Icon.Default.mergeOptions({
        iconRetinaUrl: 'https://cdnjs.cloudflare.com/ajax/libs/leaflet/1.7.1/images/marker-icon-2x.png',
        iconUrl: 'https://cdnjs.cloudflare.com/ajax/libs/leaflet/1.7.1/images/marker-icon.png',
        shadowUrl: 'https://cdnjs.cloudflare.com/ajax/libs/leaflet/1.7.1/images/marker-shadow.png',
      });
      leafletMap = L.map(mapContainer).setView([editLat || 20, editLng || 77], editLat ? 13 : 4);
      L.tileLayer('https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png', { attribution: '© OpenStreetMap' }).addTo(leafletMap);
      leafletMarker = L.marker([editLat || 20, editLng || 77], { draggable: true }).addTo(leafletMap);
      leafletMarker.on('dragend', () => { const p = leafletMarker.getLatLng(); editLat = p.lat; editLng = p.lng; });
      leafletMap.on('click', (e: any) => { leafletMarker.setLatLng(e.latlng); editLat = e.latlng.lat; editLng = e.latlng.lng; });
    } else if (leafletMap) {
      leafletMap.setView([editLat || 20, editLng || 77], editLat ? 13 : 4);
      leafletMarker.setLatLng([editLat || 20, editLng || 77]);
    }
  }

  async function searchLocation() {
    if (!locationSearchQuery) return;
    try {
      const res = await fetch(`https://nominatim.openstreetmap.org/search?format=json&q=${encodeURIComponent(locationSearchQuery)}`);
      const data = await res.json();
      if (data?.length > 0) {
        const lat = parseFloat(data[0].lat);
        const lon = parseFloat(data[0].lon);
        editLat = lat; editLng = lon;
        if (leafletMap && leafletMarker) {
          leafletMap.setView([lat, lon], 13);
          leafletMarker.setLatLng([lat, lon]);
        }
      }
    } catch (e) { console.error(e); }
  }

  async function saveLocation() {
    try {
      const res: [string, string] | null = await invoke('update_image_metadata', {
        path: photo.path, newDate: null, newLat: editLat, newLng: editLng
      });
      photo = { ...photo, lat: editLat, lng: editLng, city: res?.[0] ?? photo.city, country: res?.[1] ?? photo.country };
      onUpdated(photo);
      isEditingLocation = false;
    } catch (e) { console.error(e); }
  }
</script>

<div class="info-panel">
  <div class="panel-header">
    <h2>Details</h2>
    <button class="close-btn" on:click={onClose} aria-label="Close info panel">
      <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
    </button>
  </div>

  <!-- File Name -->
  <div class="info-section">
    <div class="section-title">File</div>
    <div class="info-row"><span class="info-val">{photo.path.split('.').pop()?.toUpperCase()} {photo.hdr ? ' · HDR' : ''}{photo.color_profile ? ` · ${photo.color_profile}` : ''}</span></div>
    {#if photo.media_subtype === 'live' || photo.media_subtype === 'motion_embedded'}<div class="info-row"><span class="info-val">Live Photo{photo.anim_duration_ms ? ` · ${(photo.anim_duration_ms/1000).toFixed(1)} s` : ''}{pairedFile ? ` · with ${pairedFile}` : ''}</span></div>{/if}
    {#if photo.media_subtype === 'animated'}<div class="info-row"><span class="info-val">Animated · {photo.frame_count ?? '?'} frames{photo.anim_duration_ms ? ` · ${(photo.anim_duration_ms/1000).toFixed(1)} s` : ''}</span></div>{/if}
    <div class="info-row">
      <span class="info-key">Name</span>
      {#if isEditingFileName}
        <div class="edit-row">
          <input class="edit-input" type="text" bind:value={editFileNameValue} on:keydown={e => e.key === 'Enter' && saveFileName()} />
          <button class="save-btn" on:click={saveFileName}>Save</button>
          <button class="cancel-btn" on:click={() => isEditingFileName = false}>Cancel</button>
        </div>
      {:else}
        <span class="info-val filename">
          {photo.path.split('/').pop() || 'Unknown'}
          <button class="edit-icon" aria-label="Rename" on:click={() => { isEditingFileName = true; editFileNameValue = photo.path.split('/').pop() || ''; }}>
            <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z"/></svg>
          </button>
        </span>
      {/if}
    </div>
    {#if photo.size}
      <div class="info-row">
        <span class="info-key">Size</span>
        <span class="info-val">{formatBytes(photo.size)}</span>
      </div>
    {/if}
    {#if photo.width && photo.height}
      <div class="info-row">
        <span class="info-key">Dimensions</span>
        <span class="info-val">{photo.width} × {photo.height}</span>
      </div>
    {/if}
    {#if photo.media_type === 'video' && photo.duration}
      <div class="info-row">
        <span class="info-key">Duration</span>
        <span class="info-val">{formatDuration(photo.duration)}</span>
      </div>
    {/if}
    {#each streamList as row (row.label)}
      <div class="info-row"><span class="info-key">{row.label}</span><span class="info-val">{row.value}</span></div>
    {/each}
  </div>

  {#if camera || cameraList.length}
    <div class="info-section">
      <div class="section-title">Camera</div>
      {#if camera}<div class="info-row"><span class="info-val camera-name">Shot on {camera}</span></div>{/if}
      {#each cameraList as row (row.label)}
        <div class="info-row"><span class="info-key">{row.label}</span><span class="info-val">{row.value}</span></div>
      {/each}
    </div>
  {/if}

  <!-- Date -->
  <div class="info-section">
    <div class="section-title">Date</div>
    {#if isEditingDate}
      <div class="edit-col">
        <input class="edit-input" type="datetime-local" bind:value={editDateValue} />
        <div class="edit-actions">
          <button class="save-btn" on:click={saveDate}>Save</button>
          <button class="cancel-btn" on:click={() => isEditingDate = false}>Cancel</button>
        </div>
      </div>
    {:else}
      <div class="info-row">
        <span class="info-val date-val">
          {formatDayDate(photo.mtime)}, {formatTime(photo.mtime)}{zone ? ` · ${zone}` : ''}
          <button class="edit-icon" aria-label="Edit date" on:click={() => {
            isEditingDate = true;
            editDateValue = new Date(photo.mtime * 1000 - (new Date().getTimezoneOffset() * 60000)).toISOString().slice(0, 16);
          }}>
            <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z"/></svg>
          </button>
        </span>
      </div>
    {/if}
  </div>

  <!-- Location -->
  <div class="info-section">
    <div class="section-title">Location</div>
    {#if isEditingLocation}
      <div class="location-editor">
        <div class="loc-search-row">
          <input class="edit-input" type="text" bind:value={locationSearchQuery} placeholder="Search place..." on:keydown={e => e.key === 'Enter' && searchLocation()} />
          <button class="search-btn" on:click={searchLocation}>Search</button>
        </div>
        <div bind:this={mapContainer} class="map-container"></div>
        <div class="edit-actions">
          <button class="save-btn" on:click={saveLocation}>Save</button>
          <button class="cancel-btn" on:click={() => isEditingLocation = false}>Cancel</button>
        </div>
      </div>
    {:else}
      <div class="info-row">
        <span class="info-val">
          {#if photo.city}
            {photo.city}, {photo.country}
          {:else if photo.lat}
            {photo.lat?.toFixed(4)}, {photo.lng?.toFixed(4)}
          {/if}
          {#if photo.city || photo.lat}
            <button class="edit-icon" aria-label="Edit location" on:click={startEditLocation}>
              <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z"/></svg>
            </button>
          {:else}
            <button class="add-btn" on:click={startEditLocation}>Add location</button>
          {/if}
        </span>
      </div>
      {#each placeList as row (row.label)}
        <div class="info-row"><span class="info-key">{row.label}</span><span class="info-val">{row.value}</span></div>
      {/each}
    {/if}
  </div>

  {#if aboutList.length}
    <div class="info-section">
      <div class="section-title">Description</div>
      {#each aboutList as row (row.label)}
        <div class="info-row"><span class="info-key">{row.label}</span><span class="info-val">{row.value}</span></div>
      {/each}
    </div>
  {/if}

  <!-- Path -->
  <div class="info-section">
    <div class="section-title">Path</div>
    <div class="info-row">
      <span class="info-val path-text">{photo.path}</span>
    </div>
  </div>
</div>

<style>
  .info-panel {
    width: 340px;
    min-width: 340px;
    height: 100%;
    background: var(--surface);
    color: var(--text);
    border-left: 1px solid var(--hairline);
    display: flex;
    flex-direction: column;
    overflow-y: auto;
    box-sizing: border-box;
    font-family: var(--font);
  }
  .panel-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 20px 24px 16px;
    border-bottom: 1px solid var(--hairline);
    position: sticky;
    top: 0;
    background: var(--surface);
    z-index: 1;
  }
  .panel-header h2 { margin: 0; font-size: var(--fs-day); font-weight: 600; }
  .close-btn, .edit-icon {
    background: none;
    border: 0;
    cursor: pointer;
    color: var(--muted);
    border-radius: var(--radius-control);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font: inherit;
  }
  .close-btn { padding: 6px; }
  .close-btn:hover, .edit-icon:hover { background: var(--surface-hover); color: var(--text); }
  .close-btn:focus-visible, .edit-icon:focus-visible { outline: 2px solid var(--text); outline-offset: 1px; opacity: 1; }

  .info-section { padding: 18px 24px; border-bottom: 1px solid var(--hairline); }
  .section-title {
    font-size: var(--fs-label);
    font-weight: 500;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    margin-bottom: 10px;
  }
  .info-row { display: flex; align-items: flex-start; gap: 8px; margin-bottom: 6px; }
  .info-key { font-size: var(--fs-meta); color: var(--muted); width: 104px; flex-shrink: 0; padding-top: 1px; }
  .info-val {
    font-size: var(--fs-body);
    color: var(--text);
    word-break: break-word;
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 1;
  }
  .filename { word-break: break-all; }
  .date-val { align-items: flex-start; flex-direction: column; gap: 4px; }
  .path-text { font-size: var(--fs-meta); color: var(--muted); word-break: break-all; }
  .camera-name { font-weight: 500; }
  .add-btn {
    background: none;
    border: 1px dashed var(--hairline);
    border-radius: var(--radius-control);
    color: var(--muted);
    cursor: pointer;
    font: inherit;
    font-size: var(--fs-meta);
    padding: 5px 10px;
  }
  .add-btn:hover { color: var(--text); border-color: var(--muted); }
  .add-btn:focus-visible { outline: 2px solid var(--text); outline-offset: 1px; }

  .edit-icon { opacity: 0; transition: opacity 0.15s; flex-shrink: 0; padding: 3px; }
  .info-row:hover .edit-icon, .info-section:hover .edit-icon { opacity: 1; }

  .edit-col { display: flex; flex-direction: column; gap: 8px; }
  .edit-row { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .edit-input {
    background: var(--search-bg);
    border: 1px solid var(--hairline);
    border-radius: var(--radius-control);
    padding: 7px 10px;
    font: inherit;
    font-size: var(--fs-body);
    color: var(--text);
    outline: none;
    width: 100%;
    box-sizing: border-box;
    color-scheme: inherit;
  }
  .edit-input:focus { border-color: var(--muted); }
  .edit-actions { display: flex; gap: 6px; }
  .save-btn, .cancel-btn, .search-btn {
    font: inherit;
    font-size: var(--fs-meta);
    border-radius: var(--radius-control);
    padding: 6px 12px;
    cursor: pointer;
    white-space: nowrap;
    border: 1px solid var(--hairline);
    background: var(--bg);
    color: var(--text);
  }
  .save-btn { background: var(--text); color: var(--bg); border-color: var(--text); font-weight: 500; }
  .save-btn:hover { opacity: 0.85; }
  .cancel-btn:hover, .search-btn:hover { background: var(--surface-hover); }
  .location-editor { display: flex; flex-direction: column; gap: 10px; }
  .loc-search-row { display: flex; gap: 6px; }
  .loc-search-row .edit-input { flex: 1; }
  .map-container { width: 100%; height: 180px; overflow: hidden; border: 1px solid var(--hairline); }
</style>
