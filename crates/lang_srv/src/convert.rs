use roc_region::all::{LineColumn, LineColumnRegion, LineInfo, Region};
use tower_lsp::lsp_types::{Position, Range};

pub(crate) trait ToRange {
    type Feed;

    fn to_range(&self, feed: &Self::Feed) -> Range;
}

impl ToRange for Region {
    type Feed = LineInfo;

    fn to_range(&self, line_info: &LineInfo) -> Range {
        let LineColumnRegion { start, end } = line_info.convert_region(*self);
        Range {
            start: Position {
                line: start.line,
                character: start.column,
            },
            end: Position {
                line: end.line,
                character: end.column,
            },
        }
    }
}

pub(crate) trait ToRegion {
    type Feed;

    fn to_region(&self, feed: &Self::Feed) -> Region;
}

impl ToRegion for Range {
    type Feed = LineInfo;

    fn to_region(&self, line_info: &LineInfo) -> Region {
        let lc_region = LineColumnRegion {
            start: LineColumn {
                line: self.start.line,
                column: self.start.character,
            },
            end: LineColumn {
                line: self.end.line,
                column: self.end.line,
            },
        };

        line_info.convert_line_column_region(lc_region)
    }
}

pub(crate) trait ToRocPosition {
    type Feed;

    fn to_roc_position(&self, feed: &Self::Feed) -> roc_region::all::Position;
}

impl ToRocPosition for tower_lsp::lsp_types::Position {
    type Feed = LineInfo;

    fn to_roc_position(&self, line_info: &LineInfo) -> roc_region::all::Position {
        let lc = LineColumn {
            line: self.line,
            column: self.character,
        };
        line_info.convert_line_column(lc)
    }
}

pub(crate) mod diag {
    use std::path::Path;

    use roc_load::LoadingProblem;
    use roc_region::all::LineInfo;
    use roc_solve_problem::TypeError;

    use roc_reporting::report::{RocDocAllocator, Severity};
    use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};

    use super::ToRange;

    pub trait IntoLspSeverity {
        fn into_lsp_severity(self) -> DiagnosticSeverity;
    }

    impl IntoLspSeverity for Severity {
        fn into_lsp_severity(self) -> DiagnosticSeverity {
            match self {
                Severity::RuntimeError => DiagnosticSeverity::ERROR,
                Severity::Warning => DiagnosticSeverity::WARNING,
            }
        }
    }

    pub trait IntoLspDiagnostic<'a> {
        type Feed;

        fn into_lsp_diagnostic(self, feed: &'a Self::Feed) -> Option<Diagnostic>;
    }

    impl IntoLspDiagnostic<'_> for LoadingProblem<'_> {
        type Feed = ();

        fn into_lsp_diagnostic(self, _feed: &()) -> Option<Diagnostic> {
            let range = Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: 0,
                    character: 1,
                },
            };

            let msg;
            match self {
                LoadingProblem::FileProblem { filename, error } => {
                    msg = format!(
                        "Failed to load {} due to an I/O error: {}",
                        filename.display(),
                        error
                    );
                }
                LoadingProblem::ParsingFailed(_) => {
                    unreachable!("should be formatted before sent back")
                }
                LoadingProblem::UnexpectedHeader(header) => {
                    msg = format!("Unexpected header: {}", header);
                }
                LoadingProblem::MsgChannelDied => {
                    msg = format!("Internal error: message channel died");
                }
                LoadingProblem::ErrJoiningWorkerThreads => {
                    msg = format!("Internal error: analysis worker threads died");
                }
                LoadingProblem::TriedToImportAppModule => {
                    msg = format!("Attempted to import app module");
                }
                LoadingProblem::FormattedReport(report) => {
                    msg = report;
                }
            };

            Some(Diagnostic {
                range,
                severity: Some(DiagnosticSeverity::ERROR),
                code: None,
                code_description: None,
                source: Some("load".to_owned()),
                message: msg,
                related_information: None,
                tags: None,
                data: None,
            })
        }
    }

    pub struct ProblemFmt<'a> {
        pub alloc: &'a RocDocAllocator<'a>,
        pub line_info: &'a LineInfo,
        pub path: &'a Path,
    }

    impl<'a> IntoLspDiagnostic<'a> for roc_problem::can::Problem {
        type Feed = ProblemFmt<'a>;

        fn into_lsp_diagnostic(self, fmt: &'a ProblemFmt<'a>) -> Option<Diagnostic> {
            let range = self.region().to_range(fmt.line_info);

            let report = roc_reporting::report::can_problem(
                &fmt.alloc,
                &fmt.line_info,
                fmt.path.to_path_buf(),
                self,
            );

            let severity = report.severity.into_lsp_severity();

            let mut msg = String::new();
            report.render_ci(&mut msg, fmt.alloc);

            Some(Diagnostic {
                range,
                severity: Some(severity),
                code: None,
                code_description: None,
                source: None,
                message: msg,
                related_information: None,
                tags: None,
                data: None,
            })
        }
    }

    impl<'a> IntoLspDiagnostic<'a> for TypeError {
        type Feed = ProblemFmt<'a>;

        fn into_lsp_diagnostic(self, fmt: &'a ProblemFmt<'a>) -> Option<Diagnostic> {
            let range = self.region().to_range(fmt.line_info);

            let report = roc_reporting::report::type_problem(
                &fmt.alloc,
                &fmt.line_info,
                fmt.path.to_path_buf(),
                self,
            )?;

            let severity = report.severity.into_lsp_severity();

            let mut msg = String::new();
            report.render_ci(&mut msg, fmt.alloc);

            Some(Diagnostic {
                range,
                severity: Some(severity),
                code: None,
                code_description: None,
                source: None,
                message: msg,
                related_information: None,
                tags: None,
                data: None,
            })
        }
    }
}
