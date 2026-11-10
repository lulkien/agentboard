// Agentboard dashboard: reads the daemon's read-only JSON API and builds the DOM.
// Every node is made with createElement/createTextNode, so stored text is never
// parsed as markup. Plain links and GET forms navigate; there is no client router
// and live update is an opt-in poll of the board cursor.
(() => {
  'use strict';

  const GLYPH = {
    todo: '○', planning: '◇', implementing: '▶', blocked: '■',
    reviewing: '◎', done: '✓', canceled: '✕',
  };
  const KINDS = ['created', 'updated', 'claimed', 'commented'];
  const SVG = 'http://www.w3.org/2000/svg';

  // ---- DOM helpers -------------------------------------------------------
  function append(node, kids) {
    for (const kid of kids.flat(Infinity)) {
      if (kid === null || kid === undefined || kid === false) continue;
      node.append(kid instanceof Node ? kid : document.createTextNode(String(kid)));
    }
    return node;
  }
  function make(ns, tag, attrs, kids) {
    const node = ns ? document.createElementNS(ns, tag) : document.createElement(tag);
    for (const [key, value] of Object.entries(attrs || {})) {
      if (value === null || value === undefined || value === false) continue;
      if (key === 'class') node.setAttribute('class', value);
      else node.setAttribute(key, value === true ? '' : String(value));
    }
    return append(node, kids);
  }
  const h = (tag, attrs, ...kids) => make(null, tag, attrs, kids);
  const svg = (tag, attrs, ...kids) => make(SVG, tag, attrs, kids);
  const frag = (...kids) => append(document.createDocumentFragment(), kids);

  // ---- formatting --------------------------------------------------------
  const enc = (value) => encodeURIComponent(value).replace(/[!'()*]/g,
    (c) => '%' + c.charCodeAt(0).toString(16).toUpperCase());
  const iso = (ms) => new Date(ms).toISOString();
  function age(now, ms) {
    const elapsed = Math.max(now - ms, 0);
    if (elapsed < 60000) return 'now';
    if (elapsed < 3600000) return `${Math.floor(elapsed / 60000)} min`;
    if (elapsed < 86400000) return `${Math.floor(elapsed / 3600000)} h`;
    return `${Math.floor(elapsed / 86400000)} d`;
  }
  const glyph = (status) => GLYPH[status] || '';

  // The project filter travels on every link as ?project=ID.
  const search = new URLSearchParams(location.search);
  const project = search.has('project') ? search.get('project') : null;
  function url(path) {
    if (project === null || project === '') return path;
    return `${path}${path.includes('?') ? '&' : '?'}project=${enc(project)}`;
  }
  function query(parts) {
    const pairs = parts.filter(([, value]) => value !== null && value !== undefined && value !== '');
    return pairs.map(([key, value]) => `${key}=${enc(String(value))}`).join('&');
  }
  function tasksUrl(f, offset) {
    const q = query([['status', f.status], ['assignee', f.assignee], ['parent', f.parent],
      ['offset', offset], ['project', f.project]]);
    return q ? `/tasks?${q}` : '/tasks';
  }
  function feedUrl(f, before) {
    const q = query([['task', f.task], ['kind', f.kind], ['before', before], ['project', f.project]]);
    return q ? `/feed?${q}` : '/feed';
  }

  // ---- shared pieces -----------------------------------------------------
  const badge = (status) => h('span', { class: `badge status-${status}` },
    h('span', { 'aria-hidden': 'true' }, glyph(status)), ` ${status}`);

  function strip(task, href) {
    return h('a', { class: `task-strip status-${task.status}`, href, title: task.title },
      h('span', { class: 'strip-glyph', 'aria-hidden': 'true' }, glyph(task.status)), ' ',
      h('span', { class: 'strip-id' }, `T${task.id}`), ' ',
      h('span', { class: 'strip-title' }, task.title), ' ',
      h('span', { class: 'visually-hidden' }, task.status));
  }
  const stripItem = (task) => h('li', null, strip(task, url(`/tasks/${task.id}`)));
  const projectName = (projects, id) => {
    const found = projects.find((item) => item.project_id === id);
    return found ? found.project_name : null;
  };
  function textBlock(cls, text) { return h('div', { class: cls }, text); }

  // ---- board -------------------------------------------------------------
  function card(task, projects, now) {
    const pname = project === null && task.project_id !== null
      ? projectName(projects, task.project_id) : null;
    return h('li', null,
      h('a', { class: `card status-${task.status}`, href: url(`/tasks/${task.id}`), title: task.title },
        h('span', { class: 'card-head' },
          h('span', { class: 'card-glyph', 'aria-hidden': 'true' }, glyph(task.status)), ' ',
          h('span', { class: 'card-id' }, `T${task.id}`), ' ',
          h('span', { class: 'visually-hidden' }, task.status),
          h('time', { datetime: iso(task.updated_at_ms), title: iso(task.updated_at_ms) },
            age(now, task.updated_at_ms))),
        h('span', { class: 'card-title' }, task.title),
        h('span', { class: 'card-meta' },
          h('span', { class: 'card-assignee' }, task.assignee || 'Unassigned'),
          task.parent_id !== null ? h('span', { class: 'card-parent' }, `in T${task.parent_id}`) : null,
          pname !== null ? h('span', { class: 'card-project' }, pname) : null)));
  }

  function board(d, projects) {
    const now = Date.now();
    const attention = h('section', { class: 'attention panel', 'aria-labelledby': 'attention-title' },
      h('h2', { id: 'attention-title' }, 'Needs attention ', h('span', null, d.attention.total)),
      h('ol', { class: 'strip-list' },
        d.attention.tasks.map(stripItem),
        d.attention.tasks.length === 0 ? h('li', { class: 'empty' }, 'No rows need attention') : null),
      d.attention.total > d.attention.tasks.length
        ? h('a', { href: url('/tasks') }, `+${d.attention.total - d.attention.tasks.length} more tasks`) : null);

    const names = new Set(d.agents.map((agent) => agent.name));
    for (const held of d.held) names.add(held.assignee);
    const seats = [...names].sort().map((name) => {
      const held = d.held.find((item) => item.assignee === name);
      if (held) {
        return h('li', null,
          h('a', { class: 'seat-chip', href: url(`/tasks?assignee=${enc(name)}`), title: name },
            h('span', { class: 'seat-name' }, name), ' ', h('span', { class: 'seat-count' }, held.total)));
      }
      const agent = d.agents.find((item) => item.name === name);
      const seen = agent && agent.last_seen_ms !== null ? ` ${age(now, agent.last_seen_ms)}` : '';
      return h('li', null,
        h('span', { class: 'seat-chip idle', title: name },
          h('span', { class: 'seat-name' }, name), ` idle${seen}`));
    });
    const seatBar = h('section', { class: 'seat-bar', 'aria-labelledby': 'seats-title' },
      h('h2', { id: 'seats-title' }, 'Seats'),
      h('ul', null, seats, seats.length === 0 ? h('li', { class: 'empty' }, 'No seats yet') : null));

    const columns = d.lanes.map((lane) => h('section', { class: 'column', 'aria-labelledby': `col-${lane.status}` },
      h('h2', { class: `column-head status-${lane.status}`, id: `col-${lane.status}` },
        h('span', { 'aria-hidden': 'true' }, glyph(lane.status)), ` ${lane.status} `,
        h('span', { class: 'column-count' }, lane.total)),
      h('ol', { class: 'card-list' },
        lane.tasks.map((task) => card(task, projects, now)),
        lane.tasks.length === 0 ? h('li', { class: 'empty' }, 'None') : null),
      lane.total > lane.tasks.length
        ? h('a', { class: 'column-more', href: url(`/tasks?status=${lane.status}`) },
          `+${lane.total - lane.tasks.length} more`) : null));

    const closed = ['done', 'canceled'].map((status) => {
      const entry = d.counts.find((item) => item.status === status);
      return h('a', { class: `status-chip closed-item status-${status}`, href: url(`/tasks?status=${status}`) },
        h('span', { 'aria-hidden': 'true' }, glyph(status)), ` ${status} `,
        h('span', { class: 'closed-count' }, entry ? entry.count : 0));
    });
    const aside = h('p', { class: 'closed-summary' },
      h('span', { class: 'visually-hidden' }, 'Closed rows: '), closed);
    return { title: 'Board', aside, content: frag(seatBar, attention, h('div', { class: 'kanban' }, columns)) };
  }

  // ---- epics -------------------------------------------------------------
  function epicRow(epic) {
    const task = epic.task;
    const childrenHref = url(`/tasks?parent=${task.id}`);
    const total = Math.max(epic.total, 1);
    let x = 0;
    const rects = [];
    for (const entry of epic.counts) {
      if (entry.count === 0) continue;
      const width = (entry.count / total) * 100;
      rects.push(svg('rect', { class: `status-${entry.status}`, x: x.toFixed(3), y: 0,
        width: width.toFixed(3), height: 10 }));
      x += width;
    }
    return h('article', { class: `epic-row depth-${epic.depth}` },
      h('div', { class: 'epic-heading' }, strip(task, childrenHref),
        task.parent_id !== null
          ? h('a', { class: 'meta', href: url(`/tasks/${task.parent_id}`) }, `in T${task.parent_id}`) : null,
        h('span', { class: 'meta' }, `Priority ${task.priority}`)),
      h('div', { class: 'epic-rollup' },
        svg('svg', { class: 'rollup-bar', viewBox: '0 0 100 10', role: 'img',
          'aria-label': `${epic.done} of ${epic.total} direct children done`,
          preserveAspectRatio: 'none' }, rects),
        h('span', { class: 'progress-copy' }, `${epic.done} / ${epic.total} done`),
        h('ul', { class: 'counts' }, epic.counts.filter((e) => e.count > 0).map((e) =>
          h('li', { class: `status-${e.status}` }, `${e.status}: `, h('strong', null, e.count)))),
        h('a', { href: childrenHref }, 'All children')),
      h('ul', { class: 'strip-list epic-children' }, epic.children.tasks.map(stripItem)),
      epic.children.total > epic.children.tasks.length
        ? h('p', { class: 'epic-more' },
          h('a', { href: childrenHref }, `+${epic.children.total - epic.children.tasks.length} more`)) : null);
  }

  function epics(d) {
    const content = d.epics.length === 0
      ? h('p', { class: 'empty panel' }, 'No epics yet') : d.epics.map(epicRow);
    return { title: 'Epics', content: h('section', { class: 'epic-list', 'aria-label': 'Epics' }, content) };
  }

  // ---- tasks -------------------------------------------------------------
  function taskRow(task, projects) {
    let where = '—';
    if (task.project_id !== null) where = projectName(projects, task.project_id) || String(task.project_id);
    return h('tr', null,
      h('td', null, h('a', { href: url(`/tasks/${task.id}`) }, `T${task.id}`)),
      h('td', null, badge(task.status)),
      h('td', { class: 'task-title', title: task.title }, h('span', null, task.title)),
      h('td', null, task.assignee || '—'),
      h('td', { class: 'path' }, where),
      h('td', { class: 'priority' }, task.priority));
  }

  function tasks(d, projects) {
    const f = { status: d.status, assignee: d.assignee, parent: d.parent, project: d.project };
    const total = d.counts.reduce((sum, entry) => sum + entry.count, 0);
    const current = (on) => (on ? { 'aria-current': 'true' } : {});
    const chips = h('nav', { class: 'status-chips', 'aria-label': 'Task status' },
      h('a', { class: 'status-chip', href: tasksUrl({ ...f, status: null }, null), ...current(d.status === null) },
        `All ${total}`),
      d.counts.map((entry) => h('a', { class: `status-chip status-${entry.status}`,
        href: tasksUrl({ ...f, status: entry.status }, null), ...current(d.status === entry.status) },
      `${entry.status} ${entry.count}`)));

    const offset = Math.min(d.offset, d.total);
    const start = d.tasks.length === 0 ? 0 : offset + 1;
    const pageCount = Math.max(Math.ceil(d.total / d.page_size), 1);
    const pageNumber = Math.floor(offset / d.page_size) + 1;
    const arrow = (label, rel, href, glyphText) => (href === null
      ? h('span', { class: 'page-arrow', 'aria-label': `${label} page`, 'aria-disabled': 'true' }, glyphText)
      : h('a', { class: 'page-arrow', 'aria-label': `${label} page`, rel, href }, glyphText));

    const panel = h('section', { class: 'panel' },
      h('form', { id: 'task-filter', method: 'get', action: '/tasks' },
        h('label', null, 'Status',
          h('select', { name: 'status' }, h('option', { value: '' }, 'All statuses'),
            d.counts.map((entry) => h('option', { value: entry.status,
              selected: d.status === entry.status }, entry.status)))),
        h('label', null, 'Assignee',
          h('input', { name: 'assignee', value: d.assignee || '', autocomplete: 'off' })),
        h('label', null, 'Parent ID',
          h('input', { name: 'parent', inputmode: 'numeric', value: d.parent === null ? '' : d.parent })),
        h('button', { type: 'submit' }, 'Apply filters')),
      d.project !== null
        ? h('input', { form: 'task-filter', type: 'hidden', name: 'project', value: d.project }) : null,
      d.parent !== null
        ? h('p', { class: 'filter-note' }, 'Showing direct children of epic ',
          h('a', { href: url(`/tasks?parent=${d.parent}`) }, `T${d.parent}`)) : null,
      h('p', { class: 'result-count' }, `Showing ${start}–${offset + d.tasks.length} of ${d.total} tasks`),
      h('div', { class: 'table-scroll' },
        h('table', { class: 'task-table' },
          h('thead', null, h('tr', null,
            ['ID', 'Status', 'Title', 'Assignee', 'Project', 'Priority'].map(
              (name) => h('th', { scope: 'col' }, name)))),
          h('tbody', null,
            d.tasks.length === 0
              ? h('tr', null, h('td', { colspan: '6', class: 'empty' }, 'No tasks match these filters')) : null,
            d.tasks.map((task) => taskRow(task, projects))))),
      h('nav', { class: 'pagination', 'aria-label': 'Task pages' },
        arrow('Previous', 'prev', offset > 0 ? tasksUrl(f, Math.max(offset - d.page_size, 0)) : null, '←'),
        h('span', null, `Page ${pageNumber} of ${pageCount}`),
        arrow('Next', 'next', d.more ? tasksUrl(f, offset + d.tasks.length) : null, '→')));
    return { title: 'Tasks', content: frag(chips, panel) };
  }

  // ---- task detail -------------------------------------------------------
  function show(field, value, projects) {
    if (field === 'project_id' && Number.isInteger(value)) {
      return projectName(projects, value) || String(value);
    }
    if (typeof value === 'string') return value;
    if (value === null) return '—';
    return JSON.stringify(value);
  }

  function changes(event, projects) {
    const values = event.payload.changes;
    const rows = values && typeof values === 'object' && !Array.isArray(values)
      ? Object.entries(values) : [];
    if (rows.length === 0) return h('p', { class: 'meta' }, 'Task fields updated');
    return h('ul', { class: 'change-list' }, rows.map(([field, pair]) => {
      const old = pair && 'old' in pair ? show(field, pair.old, projects) : '';
      const next = pair && 'new' in pair ? show(field, pair.new, projects) : '';
      return h('li', null, h('strong', null, field), `: ${old} → ${next}`);
    }));
  }

  function activity(events, task, projects) {
    let content;
    if (events.length === 0) {
      content = h('p', { class: 'empty' }, 'No activity yet');
    } else {
      content = h('ol', { class: 'timeline' }, events.map((event) => {
        const payload = event.payload || {};
        const text = typeof payload.text === 'string' ? payload.text
          : (typeof payload.reason === 'string' ? payload.reason : null);
        let body = null;
        if (text !== null) body = textBlock('full-text event-text', text);
        else if (event.kind === 'updated') body = changes(event, projects);
        return h('li', null,
          h('div', { class: 'timeline-heading' }, h('strong', null, event.kind),
            h('span', { class: 'meta' }, event.actor),
            h('time', { datetime: iso(event.timestamp_ms) }, iso(event.timestamp_ms))),
          body);
      }));
    }
    return h('section', { class: 'panel activity', id: 'activity' },
      h('div', { class: 'card-heading' }, h('h2', null, 'Activity'),
        h('a', { href: url(`/feed?task=${task.id}`) }, 'Older history')),
      content);
  }

  function detail(d, projects) {
    const task = d.task;
    const reports = [['analyst-report', 'Analyst report', task.analyst_report],
      ['coder-report', 'Coder report', task.coder_report],
      ['reviewer-report', 'Reviewer report', task.reviewer_report]]
      .filter(([, , text]) => text !== '');
    const onPage = [['Goal', 'goal'], ['Description', 'description'],
      ...reports.map(([id, heading]) => [heading, id]), ['Activity', 'activity']];

    let children = null;
    if (d.epic !== null) {
      const maximum = Math.max(d.epic.total, 1);
      children = h('section', { class: 'panel children-panel' },
        h('div', { class: 'card-heading' }, h('h2', null, 'Child tasks'),
          h('span', { class: 'meta' }, `${d.epic.done} / ${d.epic.total} done`)),
        h('progress', { max: maximum, value: Math.min(d.epic.done, maximum),
          'aria-label': `${task.title}: ${d.epic.done} of ${d.epic.total} direct children done` }),
        h('ul', { class: 'strip-list' }, d.children.tasks.map(stripItem)),
        d.children.total > d.children.tasks.length
          ? h('p', null, h('a', { href: url(`/tasks?parent=${task.id}`) },
            `View all ${d.children.total} child tasks`)) : null);
    }

    const row = (label, value) => h('div', { class: 'metadata-row' }, h('dt', null, label), h('dd', null, value));
    let where = '—';
    if (task.project_id !== null) {
      const found = projects.find((item) => item.project_id === task.project_id);
      where = found
        ? h('a', { href: `/tasks?project=${found.project_id}`, title: found.project_path }, found.project_name)
        : String(task.project_id);
    }
    const metadata = [
      row('Status', badge(task.status)),
      row('Priority', h('span', { class: 'priority priority-value' }, task.priority)),
      row('Assignee', task.assignee || 'Unassigned'),
      row('Reporter', task.reporter),
      task.parent_id !== null
        ? row('Parent', h('a', { href: url(`/tasks/${task.parent_id}`) }, `T${task.parent_id}`)) : null,
      row('Project', where),
      row('Revision', task.revision),
      row('Created', h('time', { datetime: iso(task.created_at_ms) }, iso(task.created_at_ms))),
      row('Updated', h('time', { datetime: iso(task.updated_at_ms) }, iso(task.updated_at_ms))),
    ];
    const text = (id, heading, body) => h('section', { class: 'panel text-section', id },
      h('h2', null, heading), textBlock('full-text', body));

    const content = frag(
      h('nav', { class: 'breadcrumbs', 'aria-label': 'Breadcrumb' },
        h('a', { href: url('/tasks') }, 'Tasks'), ' ', h('span', { 'aria-hidden': 'true' }, '/'), ' ',
        h('span', null, `T${task.id}`)),
      h('header', { class: 'issue-header' }, h('p', { class: 'meta' }, `Task T${task.id}`),
        h('div', { class: 'card-heading' }, h('h2', null, task.title), badge(task.status))),
      h('div', { class: 'detail-layout' },
        h('div', { class: 'detail-main' },
          text('goal', 'Goal', task.goal), text('description', 'Description', task.description),
          reports.map(([id, heading, body]) => text(id, heading, body)),
          activity(d.events, task, projects)),
        h('aside', { class: 'detail-aside' },
          h('nav', { class: 'panel on-this-page', 'aria-label': 'On this page' },
            h('h2', null, 'On this page'),
            h('ul', null, onPage.map(([label, id]) => h('li', null, h('a', { href: `#${id}` }, label))))),
          children,
          h('section', { class: 'panel metadata', 'aria-label': 'Task metadata' },
            h('h2', null, 'Details'), h('dl', null, metadata)))));
    return { title: `T${task.id} | ${task.title}`, active: 'tasks', content };
  }

  // ---- feed --------------------------------------------------------------
  function feedEvent(event) {
    const payload = event.payload || {};
    const content = [];
    if (typeof payload.text === 'string') {
      content.push(textBlock('full-text event-text', payload.text));
    } else {
      for (const [label, key] of [['Comment', 'comment'], ['Reason', 'reason']]) {
        if (typeof payload[key] === 'string') {
          content.push(h('p', { class: 'full-text event-text' }, h('strong', null, `${label}:`), ` ${payload[key]}`));
        }
      }
      if (payload.changes !== undefined) {
        content.push(h('pre', { class: 'full-text event-text' }, JSON.stringify(payload.changes, null, 2)));
      }
      if (content.length === 0) {
        const note = { created: 'Task created', claimed: 'Task claimed', updated: 'Task updated',
          commented: 'Comment added' }[event.kind] || '';
        content.push(h('p', { class: 'meta' }, note));
      }
    }
    return h('li', null, h('article', { class: 'feed-event' },
      h('header', { class: 'timeline-heading' },
        h('span', { class: 'badge event-kind' }, event.kind),
        h('a', { href: url(`/tasks/${event.task_id}`) }, `T${event.task_id}`),
        h('span', { class: 'meta' }, `by ${event.actor}`),
        h('time', { datetime: iso(event.timestamp_ms) }, iso(event.timestamp_ms))),
      content));
  }

  function feed(d) {
    const f = { task: d.task, kind: d.kind, project: d.project };
    const panel = h('section', { class: 'panel feed-panel' },
      h('form', { id: 'feed-filter', method: 'get', action: '/feed' },
        h('label', null, 'Task ID',
          h('input', { name: 'task', inputmode: 'numeric', value: d.task === null ? '' : d.task })),
        h('label', null, 'Event kind',
          h('select', { name: 'kind' }, h('option', { value: '' }, 'All events'),
            KINDS.map((kind) => h('option', { value: kind, selected: d.kind === kind }, kind)))),
        h('button', { type: 'submit' }, 'Apply filters')),
      h('p', { class: 'result-count' }, `${d.events.length} matching events shown`),
      d.project !== null
        ? h('input', { form: 'feed-filter', type: 'hidden', name: 'project', value: d.project }) : null,
      d.events.length === 0
        ? h('p', { class: 'empty' }, 'No matching events')
        : h('ol', { class: 'timeline feed-timeline' }, d.events.map(feedEvent)),
      h('nav', { class: 'pagination', 'aria-label': 'Feed pages' },
        d.before !== null ? h('a', { href: feedUrl(f, d.before) }, 'Older events') : null,
        h('a', { href: feedUrl(f, null) }, 'Newest')));
    return { title: 'Feed', content: panel };
  }

  // ---- shell -------------------------------------------------------------
  const PAGES = { '/': ['board', '/api/board', board, '/'], '/epics': ['epics', '/api/epics', epics, '/epics'],
    '/tasks': ['tasks', '/api/tasks', tasks, '/tasks'], '/feed': ['feed', '/api/feed', feed, '/feed'] };

  function route(path) {
    if (PAGES[path]) return PAGES[path];
    const match = /^\/tasks\/[^/]+$/.exec(path);
    return match ? ['tasks', `/api${path}`, detail, '/tasks'] : null;
  }

  async function getJson(target) {
    const response = await fetch(target, { headers: { Accept: 'application/json' } });
    if (!response.ok) {
      const error = new Error(String(response.status));
      error.status = response.status;
      throw error;
    }
    return response.json();
  }

  function setProjects(projects, action) {
    const form = document.querySelector('.project-switcher');
    form.setAttribute('action', action);
    const select = form.querySelector('select');
    while (select.options.length > 1) select.remove(1);
    for (const item of projects) {
      const option = h('option', { value: item.project_id,
        selected: project !== null && String(item.project_id) === project }, item.project_name);
      select.append(option);
    }
    if (project === null || project === '') select.selectedIndex = 0;
  }

  function setNav(active) {
    for (const link of document.querySelectorAll('[data-nav]')) {
      const name = link.getAttribute('data-nav');
      const base = { brand: '/', board: '/', epics: '/epics', tasks: '/tasks', feed: '/feed' }[name];
      link.setAttribute('href', url(base));
      if (name === 'brand') continue;
      link.classList.toggle('active', name === active);
      if (name === active) link.setAttribute('aria-current', 'page');
      else link.removeAttribute('aria-current');
    }
  }

  function render(title, aside, content) {
    document.title = `${title} | Agentboard`;
    const main = document.getElementById('main');
    main.replaceChildren(h('div', { class: 'page-head' }, h('h1', null, title), aside), content);
  }

  async function load() {
    const found = route(location.pathname);
    if (!found) return { title: 'Not found', content: h('p', { class: 'empty panel' }, 'Not found') };
    const [active, api, build, action] = found;
    const projectQuery = project === null || project === '' ? '' : `?project=${enc(project)}`;
    const [data, listing] = await Promise.all([
      getJson(api + location.search), getJson(`/api/projects${projectQuery}`)]);
    setNav(active);
    setProjects(listing.projects, action);
    const page = build(data, listing.projects);
    if (page.active) setNav(page.active);
    return page;
  }

  async function start() {
    try {
      const page = await load();
      render(page.title, page.aside || null, page.content);
    } catch (error) {
      const message = error.status === 404 ? 'Not found'
        : (error.status === 400 ? 'Invalid query' : 'Board read failed');
      render(message, null, h('p', { class: 'empty panel' }, message));
    }
  }

  // ---- live update -------------------------------------------------------
  // Off by default. When on, poll /api/health every LIVE_MS and reload the page
  // only when the board cursor moved. A failed poll keeps the page on screen.
  const LIVE_MS = 5000;

  function live() {
    const button = document.querySelector('[data-live-toggle]');
    if (!button) return;
    let on = false;
    let timer = null;
    let cursor = null;
    let busy = false;
    try { on = localStorage.getItem('agentboard-live') === 'on'; } catch (_) { /* storage can be blocked */ }
    const label = (state) => {
      button.textContent = `Live update: ${state}`;
      button.setAttribute('aria-pressed', String(on));
    };
    // A feed page behind `before=` is history, and a focused control is a user typing.
    const idle = () => !document.hidden && !busy
      && !new URLSearchParams(location.search).has('before')
      && !document.activeElement?.matches('input, select, textarea');
    const tick = async () => {
      if (!on || !idle()) return;
      busy = true;
      try {
        const seen = (await getJson('/api/health')).cursor;
        if (cursor !== null && seen !== cursor) {
          const [x, y] = [scrollX, scrollY];
          const page = await load();
          render(page.title, page.aside || null, page.content);
          scrollTo(x, y);
        }
        cursor = seen;
        label('on');
      } catch (_) {
        label('paused, retrying');
      } finally { busy = false; }
    };
    const apply = () => {
      clearInterval(timer);
      timer = null;
      cursor = null;
      label(on ? 'on' : 'off');
      if (on) { timer = setInterval(tick, LIVE_MS); tick(); }
    };
    button.addEventListener('click', () => {
      on = !on;
      try { localStorage.setItem('agentboard-live', on ? 'on' : 'off'); } catch (_) { /* storage can be blocked */ }
      apply();
    });
    document.addEventListener('visibilitychange', () => { if (!document.hidden) tick(); });
    apply();
  }

  // ---- theme toggle ------------------------------------------------------
  function theme() {
    const button = document.querySelector('[data-theme-toggle]');
    if (!button) return;
    const allowed = ['light', 'dark', 'system'];
    let current = 'system';
    try {
      const saved = localStorage.getItem('agentboard-theme');
      if (allowed.includes(saved)) current = saved;
    } catch (_) { /* storage can be blocked */ }
    const apply = () => {
      if (current === 'system') document.documentElement.removeAttribute('data-theme');
      else document.documentElement.setAttribute('data-theme', current);
      button.setAttribute('aria-pressed', String(current !== 'system'));
      button.setAttribute('aria-label', `Change theme; current theme: ${current}`);
      button.title = `Current theme: ${current}`;
    };
    apply();
    button.addEventListener('click', () => {
      current = allowed[(allowed.indexOf(current) + 1) % allowed.length];
      apply();
      try {
        if (current === 'system') localStorage.removeItem('agentboard-theme');
        else localStorage.setItem('agentboard-theme', current);
      } catch (_) { /* storage can be blocked */ }
    });
  }

  theme();
  live();
  start();
})();
