use std::fmt::Write;

use agentboard::domain::Status;
use agentboard::store::Brief;

use crate::dashboard::html;
use crate::dashboard::routes::Page;

pub(super) fn epics(brief: &Brief) -> String {
    let mut body = String::from("<section class=\"card-grid\" aria-label=\"Epics\">");
    if brief.epics.is_empty() {
        body.push_str("<p class=\"empty card\">No epics yet</p>");
    }
    for epic in &brief.epics {
        let max = epic.total.max(1);
        let value = epic.done.min(max);
        let title = html::escape(&epic.task.title);
        let _ = write!(
            body,
            "<article class=\"card epic-card\"><div class=\"card-heading\"><h2><a href=\"/tasks?parent={}\">{}</a></h2>{}</div><p class=\"meta\">T{} · priority {}</p><p class=\"progress-copy\">{} / {} done</p><progress max=\"{max}\" value=\"{value}\" aria-label=\"{}: {} of {} direct children done\"></progress><ul class=\"counts\">",
            epic.task.id,
            title,
            html::status_badge(epic.task.status),
            epic.task.id,
            epic.task.priority,
            epic.done,
            epic.total,
            title,
            epic.done,
            epic.total
        );
        for status in Status::ALL {
            let count = epic
                .counts
                .iter()
                .find_map(|(s, n)| (*s == status).then_some(n))
                .copied()
                .unwrap_or(0);
            if count > 0 {
                let _ = write!(
                    body,
                    "<li>{}: <strong>{count}</strong></li>",
                    status.as_str()
                );
            }
        }
        body.push_str("</ul><a class=\"button-link\" href=\"/tasks?parent=");
        let _ = write!(body, "{}\">View child tasks</a></article>", epic.task.id);
    }
    body.push_str("</section>");
    html::shell("Epics", Page::Epics, &body)
}
