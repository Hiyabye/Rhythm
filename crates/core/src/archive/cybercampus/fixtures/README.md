# Cybercampus fixtures

Synthetic pages that keep the element structure, class names, scripts and date
formats of eClass (`eclass.sogang.ac.kr`) pages observed on 2026-11-06 with a
Korean `Accept-Language`. Course names, keys, people, numbers and text are
invented; no page of a real account is committed.

| File | Route |
| --- | --- |
| `terms.html` | `GET /ilos/main/rg/regular_register_list_form.acl` (excerpt of its script) |
| `courses.html` | `POST /ilos/main/rg/regular_register_list.acl` |
| `notice_list.html` | `POST /ilos/cls/st/notice/notice_list.acl` |
| `notice_view.html` | `POST /ilos/cls/st/notice/notice_view_pop.acl` |
| `efile_list.html` | `POST /ilos/cls/st/co/efile_list.acl` (shape from rycont/eclass-cli tests) |
| `activity.html` | `POST /ilos/cls/st/activity/activity_list.acl` with `type=report` |
| `report_submitted.html`, `report_open.html` | `GET /ilos/cls/st/report/report_view_form.acl?RT_SEQ=` |
| `todo.html` | `POST /ilos/mp/todo_list.acl` |
