// Settings > Meetings' call policies: the default for apps not chosen for (Always, Ask or Never),
// with a warning under Always and a hint under Never, then each app the core has seen with its own
// choice. Rows as the rest of Settings: label | control | caption.
import InkBridge
import SwiftUI

struct CallPolicyRows: View {
    let calls: CallPolicyModel

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            if let failure = calls.failure {
                Text(failure)
                    .font(Typography.caption)
                    .foregroundStyle(Theme.alert)
                    .fixedSize(horizontal: false, vertical: true)
            }
            defaultRow
            appsRows
        }
        // The buttons act on the choice the dialog was shown for, whatever the binding does first.
        .confirmationDialog(
            CallPolicyModel.startOverTitle,
            isPresented: Binding(get: { calls.startingOver != nil }, set: { if !$0 { calls.cancelStartOver() } }),
            presenting: calls.startingOver
        ) { choice in
            Button("Start over and save") { calls.confirmStartOver(choice) }
            Button("Cancel", role: .cancel) { calls.cancelStartOver() }
        } message: { _ in
            Text(CallPolicyModel.startOverDetail)
        }
    }

    private var defaultRow: some View {
        SettingColumns(controlsMinimum: SettingColumnsLayout.controlsMinimum) {
            Text(CallPolicyModel.defaultTitle)
        } controls: {
            VStack(alignment: .leading, spacing: 4) {
                SegmentsOrMenu {
                    // A closure literal for the setter (Swift 6.3 crashes on some closure forms here).
                    Picker(
                        CallPolicyModel.defaultTitle,
                        selection: Binding(get: { calls.defaultPolicy ?? .ask }, set: { calls.setDefault($0) })
                    ) {
                        ForEach(CallPolicy.allCases, id: \.self) { Text($0.title).tag($0) }
                    }
                    .labelsHidden()
                    .disabled(calls.defaultPolicy == nil)
                    .accessibilityHint(CallPolicyModel.defaultCaption)
                }
                Text(CallPolicyModel.defaultCaption)
                    .font(Typography.caption)
                    .foregroundStyle(Theme.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityHidden(true)
                if let note = calls.note {
                    Text(note)
                        .font(Typography.caption)
                        .foregroundStyle(Theme.alert)
                        .fixedSize(horizontal: false, vertical: true)
                }
                switch calls.defaultPolicy {
                case .always?:
                    // It records without asking: the people on the call must be told.
                    Label {
                        Text(CallPolicyModel.alwaysWarning)
                            .fixedSize(horizontal: false, vertical: true)
                    } icon: {
                        Image(systemName: "exclamationmark.triangle")
                    }
                    .font(Typography.caption)
                    .foregroundStyle(Theme.alert)
                    .padding(.top, 2)
                case .never?:
                    Text(CallPolicyModel.neverHint)
                        .font(Typography.caption)
                        .foregroundStyle(Theme.secondaryText)
                        .fixedSize(horizontal: false, vertical: true)
                        .padding(.top, 2)
                default:
                    EmptyView()
                }
            }
        }
        .font(Typography.body)
    }

    @ViewBuilder
    private var appsRows: some View {
        let rows = calls.rows
        if let unreadable = calls.unreadable {
            Text(CallPolicyModel.unreadableLine(unreadable))
                .font(Typography.caption)
                .foregroundStyle(Theme.alert)
                .fixedSize(horizontal: false, vertical: true)
        }
        if calls.loaded && rows.isEmpty {
            SettingColumns {
                Text(CallPolicyModel.appsTitle)
            } controls: {
                Text(CallPolicyModel.noApps)
                    .font(Typography.caption)
                    .foregroundStyle(Theme.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .font(Typography.body)
        } else if !rows.isEmpty {
            Paper.Eyebrow(text: CallPolicyModel.appsTitle)
                .padding(.top, 4)
                .accessibilityAddTraits(.isHeader)
            ForEach(rows) { row in
                CallAppRow(calls: calls, row: row)
            }
        }
    }
}

/// One app: its icon and name | its choice | when it last called.
private struct CallAppRow: View {
    let calls: CallPolicyModel
    let row: CallPolicyModel.Row

    var body: some View {
        SettingColumns {
            HStack(spacing: 8) {
                icon.frame(width: 20, height: 20).accessibilityHidden(true)
                Text(row.name).lineLimit(2)
            }
        } controls: {
            VStack(alignment: .leading, spacing: 2) {
                Picker(
                    "Calls in \(row.name)",
                    selection: Binding(get: { row.choice }, set: { calls.choose($0, for: row.id, from: .settings) })
                ) {
                    ForEach(CallChoice.allCases) { Text(calls.title($0)).tag($0) }
                }
                .labelsHidden()
                .pickerStyle(.menu)
                .fixedSize()
                if let seen = CallPolicyModel.seenCaption(row.seen) {
                    Text(seen)
                        .font(Typography.caption)
                        .foregroundStyle(Theme.secondaryText)
                        .accessibilityLabel("\(row.name): \(seen)")
                }
            }
        }
        .font(Typography.body)
        .padding(.vertical, 2)
    }

    @ViewBuilder
    private var icon: some View {
        if let image = row.icon {
            Image(nsImage: image).resizable()
        } else {
            Text(String(row.name.prefix(1)))
                .font(.system(size: 10, weight: .semibold))
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(PaperPalette.chip, in: RoundedRectangle(cornerRadius: 5))
        }
    }
}
