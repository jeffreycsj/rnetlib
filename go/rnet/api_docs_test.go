package rnet

import (
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"strings"
	"testing"
)

// A type-level summary does not document field units, zero values or ownership.
// This presence gate complements, but cannot replace, semantic contract review.
func TestPublicAPIDocumentation(t *testing.T) {
	entries, err := os.ReadDir(".")
	if err != nil {
		t.Fatal(err)
	}
	for _, entry := range entries {
		name := entry.Name()
		if !strings.HasSuffix(name, ".go") || strings.HasSuffix(name, "_test.go") {
			continue
		}
		fset := token.NewFileSet()
		file, err := parser.ParseFile(fset, name, nil, parser.ParseComments)
		if err != nil {
			t.Fatal(err)
		}
		check := func(id *ast.Ident, docs ...*ast.CommentGroup) {
			if !id.IsExported() {
				return
			}
			for _, doc := range docs {
				if doc != nil && strings.TrimSpace(doc.Text()) != "" {
					return
				}
			}
			t.Errorf("%s: missing public contract for %s", fset.Position(id.Pos()), id.Name)
		}
		for _, declaration := range file.Decls {
			switch decl := declaration.(type) {
			case *ast.FuncDecl:
				check(decl.Name, decl.Doc)
			case *ast.GenDecl:
				var singleDoc *ast.CommentGroup
				if len(decl.Specs) == 1 {
					singleDoc = decl.Doc
				}
				for _, spec := range decl.Specs {
					switch item := spec.(type) {
					case *ast.TypeSpec:
						check(item.Name, item.Doc, singleDoc)
						if fields, ok := item.Type.(*ast.StructType); ok && item.Name.IsExported() {
							for _, field := range fields.Fields.List {
								for _, id := range field.Names {
									check(id, field.Doc, field.Comment)
								}
							}
						}
					case *ast.ValueSpec:
						for _, id := range item.Names {
							check(id, item.Doc, item.Comment, singleDoc)
						}
					}
				}
			}
		}
	}
}
