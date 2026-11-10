use std::fmt::Write;

use agentboard::domain::{Project, Status, Task};
use agentboard::store::TaskPage;

use super::PAGE_SIZE;
use crate::dashboard::html;
use crate::dashboard::routes::{Filters, Page};

pub(super) fn tasks(page: &TaskPage, filters: &Filters, projects: &[Project]) -> String {
    let mut body = String::from(
        "<section class=\"panel\"><form id=\"task-filter\" method=\"get\" action=\"/tasks\"><label>Status<select name=\"status\"><option value=\"\">All statuses</option>",
    );
    for status in Status::ALL {
        let selected = filters.status == Some(status);
        let _ = write!(
            body,
            "<option value=\"{}\"{}>{}</option>",
            status.as_str(),
            if selected { " selected" } else { "" },
            status.as_str()
        );
    }
    let assignee = html::escape(filters.assignee.as_deref().unwrap_or_default());
    let parent = filters.parent.map(|id| id.to_string()).unwrap_or_default();
    let _ = write!(
        body,
        "</select></label><label>Assignee<input name=\"assignee\" value=\"{assignee}\" autocomplete=\"off\"></label><label>Parent ID<input name=\"parent\" inputmode=\"numeric\" value=\"{parent}\"></label><button type=\"submit\">Apply filters</button></form>"
    );
    if let Some(project) = filters.project {
        let _ = write!(
            body,
            "<input form=\"task-filter\" type=\"hidden\" name=\"project\" value=\"{project}\">"
        );
    }
    if let Some(id) = filters.parent {
        let _ = write!(
            body,
            "<p class=\"filter-note\">Showing direct children of epic <a href=\"{}\">T{id}</a></p>",
            html::escape(&html::url(&format!("/tasks?parent={id}"), filters.project))
        );
    }
    let offset = filters.offset.min(page.total);
    let start = if page.tasks.is_empty() { 0 } else { offset + 1 };
    let _ = write!(
        body,
        "<p class=\"result-count\">Showing {start}–{} of {} tasks</p><div class=\"table-scroll\"><table><thead><tr><th scope=\"col\">ID</th><th scope=\"col\">Status</th><th scope=\"col\">Title</th><th scope=\"col\">Assignee</th><th scope=\"col\">Project</th><th scope=\"col\">Priority</th></tr></thead><tbody>",
        offset + page.tasks.len(),
        page.total
    );
    if page.tasks.is_empty() {
        body.push_str(
            "<tr><td colspan=\"6\" class=\"empty\">No tasks match these filters</td></tr>",
        );
    }
    for task in &page.tasks {
        render_task_row(&mut body, task, filters.project);
    }
    body.push_str("</tbody></table></div><nav class=\"pagination\" aria-label=\"Task pages\">");
    if offset > 0 {
        let _ = write!(
            body,
            "<a href=\"{}\">Previous</a>",
            html::escape(&html::query_url(
                filters,
                Some(offset.saturating_sub(PAGE_SIZE))
            ))
        );
    }
    if page.more {
        let _ = write!(
            body,
            "<a href=\"{}\">Next</a>",
            html::escape(&html::query_url(
                filters,
                Some(offset.saturating_add(page.tasks.len()))
            ))
        );
    }
    body.push_str("</nav></section>");
    html::shell("Tasks", Page::Tasks, &body, filters.project, projects)
}

fn render_task_row(out: &mut String, task: &Task, project_scope: Option<u64>) {
    let assignee = html::escape(task.assignee.as_deref().unwrap_or("—"));
    let project = task
        .project_id
        .map_or_else(|| "—".to_owned(), |id| id.to_string());
    let title = html::escape(&task.title);
    let href = html::escape(&html::url(&format!("/tasks/{}", task.id), project_scope));
    let _ = write!(
        out,
        "<tr><td><a href=\"{href}\">T{}</a></td><td>{}</td><td>{title}</td><td>{assignee}</td><td class=\"path\">{project}</td><td class=\"priority\">{}</td></tr>",
        task.id,
        html::status_badge(task.status),
        task.priority
    );
}
