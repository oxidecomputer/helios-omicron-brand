TOP =		$(PWD)

PUBLISHER =	helios-dev

PROTO =		$(TOP)/proto
PKGDIR =	$(TOP)/packages
REPO =		$(PKGDIR)/repo

BRAND =		omicron1
BRANDDIR =	usr/lib/brand/$(BRAND)
MAN5DIR =	usr/share/man/man5
SMFDIR =	lib/svc/manifest/system/omicron

DIRS_0 =	$(BRANDDIR) \
		$(MAN5DIR) \
		$(SMFDIR)
DIRS =		$(DIRS_0:%=$(PROTO)/%)

SMF_0 =		baseline.xml
SMF =		$(SMF_0:%=$(PROTO)/$(SMFDIR)/%)

BINS_0 =	baseline \
		brand
BINS =		$(BINS_0:%=$(PROTO)/$(BRANDDIR)/%)

MAN5_0 =	$(BRAND).5
MAN5 =		$(MAN5_0:%=$(PROTO)/$(MAN5DIR)/%)

XML_0 =		config.xml \
		platform.xml
XML =		$(XML_0:%=$(PROTO)/$(BRANDDIR)/%)

PACKAGES_0 =	brand \
		baseline
PACKAGES =	$(PACKAGES_0:%=pkg.%)

COMMIT_COUNT =	$(shell git rev-list --count HEAD)

.PHONY: all
all: $(DIRS) $(BINS) $(MAN5) $(SMF) $(XML)

.PHONY: package
package: all $(PKGDIR) $(PACKAGES)

.PHONY: pkg.%
pkg.%: $(PKGDIR)/%.final.p5m | $(REPO)
	pkgsend publish -d $(PROTO) -s $(REPO) $<

.PRECIOUS: $(PKGDIR)/%.base.p5m
$(PKGDIR)/%.base.p5m: pkg/%.p5m | $(PKGDIR)
	@rm -f $@
	sed -e 's/%PUBLISHER%/$(PUBLISHER)/g' \
	    -e 's/%COMMIT_COUNT%/$(COMMIT_COUNT)/g' \
	    $< | pkgmogrify -v -O $@

.PRECIOUS: $(PKGDIR)/%.generate.p5m
$(PKGDIR)/%.generate.p5m: $(PKGDIR)/%.base.p5m
	@rm -f $@
	pkgdepend generate -d $(PROTO) $< > $@

.PRECIOUS: $(PKGDIR)/%.resolve.p5m
$(PKGDIR)/%.generate.p5m.resolve.p5m: $(PKGDIR)/%.generate.p5m
	@rm -f $@
	pkgdepend resolve -d $(PKGDIR) -s resolve.p5m $<

.PRECIOUS: $(PKGDIR)/%.final.p5m
$(PKGDIR)/%.final.p5m: $(PKGDIR)/%.base.p5m $(PKGDIR)/%.generate.p5m.resolve.p5m
	@rm -f $@
	cat $^ > $@

$(REPO):
	pkgrepo create $@
	pkgrepo add-publisher -s $@ $(PUBLISHER)

.PHONY: archive
archive: $(PKGDIR)/omicron-brand-1.0.$(COMMIT_COUNT).p5p | $(REPO)

$(PKGDIR)/omicron-brand-1.0.$(COMMIT_COUNT).p5p:
	@rm -f $@
	pkgrecv -a -d $@ -s $(REPO) -v -m latest '*'

$(PROTO)/$(MAN5DIR)/%: man/%
	@rm -f $@
	cp $< $@

$(PROTO)/$(SMFDIR)/%: smf/%
	@rm -f $@
	cp $< $@

$(PROTO)/$(BRANDDIR)/%.xml: config/%.xml
	@rm -f $@
	cp $< $@

$(DIRS) $(PKGDIR):
	mkdir -p $@

.PHONY: $(BINS)
$(BINS): | $(DIRS)
	cargo build --release
	cp target/release/$(@F) $@

.PHONY: readme
readme:
	rm -f _README.md
	awk '/MANUAL END/ { print; q = 0; next; }			\
	    !q { print; }						\
	    /MANUAL START/ {						\
	        q = 1;							\
	        printf("```\n");					\
	        system("mandoc -T utf8 -O width=72 man/omicron1.5 |	\
	            col -bx");						\
	        printf("```\n");					\
	        next;							\
	    }' README.md > _README.md
	mv _README.md README.md

.PHONY: pkgfmt
pkgfmt: $(PACKAGES_0:%=pkgfmt.%)
	@echo ok

.PHONY: pkgfmt.%
pkgfmt.%:
	pkgfmt -f v2 $(@:pkgfmt.%=pkg/%.p5m)

clean:
	rm -rf $(PROTO)

clobber: clean
	rm -rf $(PKGDIR)
