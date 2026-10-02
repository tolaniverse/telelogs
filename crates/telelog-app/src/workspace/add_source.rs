use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use telelog_core::SourceKind;

use super::sidebar::{KINDS, kind_icon, kind_label};
use super::{Workspace, modal};
use crate::client::BACKLOG;
use crate::theme::{MONO, Tokens, tokens};
use crate::ui::widgets::{outline_button, primary_button};
use crate::ui::{Icon, icon};

/// Form fields for the Add source dialog.
pub struct Inputs {
    docker_endpoint: Entity<InputState>,
    docker_labels: Entity<InputState>,
    kube_context: Entity<InputState>,
    kube_namespaces: Entity<InputState>,
    kube_selector: Entity<InputState>,
    vm_host: Entity<InputState>,
    vm_files: Entity<InputState>,
    backlog: Entity<InputState>,
    /// Re-renders the dialog as the Kubernetes fields change, so the command stays current.
    _subscriptions: Vec<Subscription>,
}

impl Inputs {
    pub fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let mut input = |placeholder: &str, value: &str| {
            let (placeholder, value) = (placeholder.to_string(), value.to_string());
            cx.new(|cx| {
                let mut state = InputState::new(window, cx).placeholder(placeholder);
                state.set_value(value, window, cx);
                state
            })
        };
        let docker_endpoint = input("unix:///var/run/docker.sock", "unix:///var/run/docker.sock");
        let docker_labels = input("com.docker.compose.project=shop", "");
        let kube_context = input("current context", "");
        let kube_namespaces = input("all namespaces", "");
        let kube_selector = input("app.kubernetes.io/part-of=shop", "");
        let vm_host = input("deploy@edge-03.internal", "");
        let vm_files = input("/var/log/syslog, /var/log/nginx/*.log", "");
        let backlog = input("500", &BACKLOG.to_string());
        let _subscriptions = [&kube_context, &kube_namespaces, &kube_selector]
            .into_iter()
            .map(|input| {
                cx.subscribe(input, |_, _, event, cx| {
                    if let InputEvent::Change = event {
                        cx.notify();
                    }
                })
            })
            .collect();
        Inputs {
            docker_endpoint,
            docker_labels,
            kube_context,
            kube_namespaces,
            kube_selector,
            vm_host,
            vm_files,
            backlog,
            _subscriptions,
        }
    }
}

fn kind_description(kind: SourceKind) -> &'static str {
    match kind {
        SourceKind::Docker => "Containers on a daemon",
        SourceKind::Kubernetes => "Pods across namespaces",
        SourceKind::Vm => "Files over SSH",
    }
}

/// The `telelog-server` command for the Kubernetes fields. Sources are configured on the
/// server, so the dialog hands over the command rather than pretending to connect.
pub fn kube_command(context: &str, namespaces: &str, selector: &str) -> String {
    let mut command = String::from("telelog-server --kubernetes");
    let context = context.trim();
    if !context.is_empty() {
        command.push_str(&format!(" --kube-context {}", shell_quote(context)));
    }
    let namespaces: Vec<&str> = namespaces.split(',').map(str::trim).filter(|n| !n.is_empty()).collect();
    if !namespaces.is_empty() {
        command.push_str(&format!(" --namespace {}", shell_quote(&namespaces.join(","))));
    }
    let selector = selector.trim();
    if !selector.is_empty() {
        command.push_str(&format!(" --selector {}", shell_quote(selector)));
    }
    command
}

/// Leaves plain words alone and single-quotes anything a shell would split or expand.
fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_.,/:=@".contains(c))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

fn field(t: Tokens, label: &'static str, hint: Option<&'static str>, input: &Entity<InputState>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            div()
                .flex()
                .gap(px(4.))
                .text_size(px(12.5))
                .text_color(t.fg2)
                .child(label)
                .children(hint.map(|h| div().text_color(t.fg3).child(h))),
        )
        .child(
            div()
                .h(px(36.))
                .px(px(12.))
                .flex()
                .items_center()
                .rounded(px(8.))
                .border_1()
                .border_color(t.line2)
                .bg(t.bg)
                .font_family(MONO)
                .text_size(px(12.5))
                .child(div().flex_1().child(Input::new(input).appearance(false))),
        )
}

impl Workspace {
    pub(super) fn render_add_source(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = tokens(cx);
        let kind = self.add_kind;
        let inputs = &self.add_inputs;

        let kinds = div()
            .px(px(22.))
            .py(px(18.))
            .flex()
            .gap(px(10.))
            .children(KINDS.into_iter().map(|k| {
                let selected = k == kind;
                div()
                    .id(SharedString::from(format!("kind-{}", k.as_str())))
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .p(px(14.))
                    .rounded(px(10.))
                    .border_1()
                    .border_color(if selected { t.fg } else { t.line2 })
                    .bg(t.bg)
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.add_kind = k;
                        cx.notify();
                    }))
                    .child(icon(kind_icon(k), 22., t.fg))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .text_size(px(13.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(kind_label(k)),
                            )
                            .child(div().text_size(px(12.)).text_color(t.fg3).child(kind_description(k))),
                    )
            }));

        let kube_cmd = kube_command(
            &inputs.kube_context.read(cx).value(),
            &inputs.kube_namespaces.read(cx).value(),
            &inputs.kube_selector.read(cx).value(),
        );
        let form = div()
            .px(px(22.))
            .pb(px(20.))
            .flex()
            .flex_col()
            .gap(px(14.))
            .map(|el| match kind {
                SourceKind::Docker => el
                    .child(field(t, "Docker endpoint", None, &inputs.docker_endpoint))
                    .child(field(t, "Label filter", Some("(optional)"), &inputs.docker_labels)),
                SourceKind::Kubernetes => el
                    .child(field(t, "Kubeconfig context", Some("(optional)"), &inputs.kube_context))
                    .child(field(
                        t,
                        "Namespaces",
                        Some("(optional, comma separated)"),
                        &inputs.kube_namespaces,
                    ))
                    .child(field(t, "Label selector", Some("(optional)"), &inputs.kube_selector))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(div().text_size(px(12.5)).text_color(t.fg2).child("Run the server with"))
                            .child(
                                div()
                                    .px(px(12.))
                                    .py(px(10.))
                                    .rounded(px(8.))
                                    .bg(t.bg2)
                                    .border_1()
                                    .border_color(t.line)
                                    .font_family(MONO)
                                    .text_size(px(12.))
                                    .child(kube_cmd.clone()),
                            ),
                    ),
                SourceKind::Vm => el.child(field(t, "Host", None, &inputs.vm_host)).child(field(
                    t,
                    "Files to tail",
                    Some("(comma separated, globs allowed)"),
                    &inputs.vm_files,
                )),
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(div().text_size(px(13.)).child("Backlog per target"))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(t.fg3)
                                    .child("Historical lines sent before following."),
                            ),
                    )
                    .child(
                        div()
                            .w(px(80.))
                            .h(px(32.))
                            .px(px(10.))
                            .flex()
                            .items_center()
                            .rounded(px(8.))
                            .border_1()
                            .border_color(t.line2)
                            .font_family(MONO)
                            .text_size(px(12.5))
                            .child(Input::new(&inputs.backlog).appearance(false)),
                    ),
            );

        // Sources are configured on the server for now; the dialog says so rather than pretending.
        let (message, primary) = match kind {
            SourceKind::Docker => (
                "telelog-server already tails its local Docker daemon.",
                primary_button(t, "add-done", "Done", None)
                    .on_click(cx.listener(|this, _, window, cx| this.close_overlays(window, cx)))
                    .into_any_element(),
            ),
            SourceKind::Kubernetes => (
                "Configured on the server.",
                primary_button(t, "add-copy", "Copy command", Some(Icon::Copy))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(kube_cmd.clone()));
                        this.close_overlays(window, cx);
                    }))
                    .into_any_element(),
            ),
            SourceKind::Vm => (
                "VM sources are coming soon.",
                primary_button(t, "add-connect", "Connect", None)
                    .opacity(0.5)
                    .cursor_default()
                    .into_any_element(),
            ),
        };

        let panel = div()
            .w(px(560.))
            .rounded(px(12.))
            .border_1()
            .border_color(t.line2)
            .bg(t.bg)
            .shadow_2xl()
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .px(px(22.))
                    .pt(px(20.))
                    .flex()
                    .items_start()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(
                                div()
                                    .text_size(px(17.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Add source"),
                            )
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(t.fg2)
                                    .child(format!("Connect a runtime to {}.", self.server_addr())),
                            ),
                    )
                    .child(
                        crate::ui::widgets::icon_button(t, "close-add", Icon::X, 15.)
                            .on_click(cx.listener(|this, _, window, cx| this.close_overlays(window, cx))),
                    ),
            )
            .child(kinds)
            .child(form)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(22.))
                    .py(px(14.))
                    .border_t_1()
                    .border_color(t.line)
                    .bg(t.bg2)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(12.5))
                            .text_color(t.fg3)
                            .child(icon(Icon::LockSimple, 14., t.fg3))
                            .child(message),
                    )
                    .child(
                        outline_button(t, "add-cancel", "Cancel", None)
                            .h(px(34.))
                            .px(px(14.))
                            .text_size(px(13.5))
                            .rounded(px(8.))
                            .on_click(cx.listener(|this, _, window, cx| this.close_overlays(window, cx))),
                    )
                    .child(primary),
            );
        modal(t, panel, None, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::kube_command;

    #[test]
    fn builds_the_server_command() {
        assert_eq!(kube_command("", "", ""), "telelog-server --kubernetes");
        assert_eq!(
            kube_command(" prod-eu ", "shop, payments ,", "app=api"),
            "telelog-server --kubernetes --kube-context prod-eu --namespace shop,payments --selector app=api"
        );
        assert_eq!(
            kube_command("", "", "tier in (web,api)"),
            "telelog-server --kubernetes --selector 'tier in (web,api)'"
        );
        assert_eq!(
            kube_command("it's", "", ""),
            "telelog-server --kubernetes --kube-context 'it'\\''s'"
        );
    }
}
