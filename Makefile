#
# Copyright 2023 Oxide Computer Company
#

TOP =		$(PWD)

PUBLISHER =	helios-dev

PROTO =		$(TOP)/proto
PKGDIR =	$(TOP)/packages
REPO =		$(PKGDIR)/repo

BRAND =		omicron1
BRANDDIR =	usr/lib/brand/$(BRAND)
MAN7DIR =	usr/share/man/man7
SMFDIR =	lib/svc/manifest/system/omicron

DIRS_0 =	$(BRANDDIR) \
		$(MAN7DIR) \
		$(SMFDIR)
DIRS =		$(DIRS_0:%=$(PROTO)/%)

SMF_0 =		baseline.xml
SMF =		$(SMF_0:%=$(PROTO)/$(SMFDIR)/%)

BINS_0 =	baseline \
		brand
BINS =		$(BINS_0:%=$(PROTO)/$(BRANDDIR)/%)

MAN7_0 =	$(BRAND).7
MAN7 =		$(MAN7_0:%=$(PROTO)/$(MAN7DIR)/%)

XML_0 =		config.xml \
		platform.xml
XML =		$(XML_0:%=$(PROTO)/$(BRANDDIR)/%)

PACKAGES_0 =	brand \
		baseline \
		incorporation
PACKAGES =	$(PACKAGES_0:%=pkg.%)

COMMIT_COUNT =	$(shell git rev-list --count HEAD)

.PHONY: all
all: $(DIRS) $(BINS) $(MAN7) $(SMF) $(XML)

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

$(PROTO)/$(MAN7DIR)/%: man/%
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
	cargo build --release --locked
	cp target/release/$(@F) $@

.PHONY: readme
readme:
	rm -f _README.md
	awk '/MANUAL END/ { print; q = 0; next; }			\
	    !q { print; }						\
	    /MANUAL START/ {						\
	        q = 1;							\
	        printf("```\n");					\
	        system("mandoc -T utf8 -O width=72 man/omicron1.7 |	\
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

check-xml:
	xmllint --dtdvalid /usr/share/lib/xml/dtd/brand.dtd.1  \
	    config/config.xml
	xmllint --dtdvalid /usr/share/lib/xml/dtd/zone_platform.dtd.1 \
	    config/platform.xml
	xmllint --dtdvalid /usr/share/lib/xml/dtd/service_bundle.dtd.1 \
	    config/profile.xml
	xmllint --dtdvalid /usr/share/lib/xml/dtd/service_bundle.dtd.1 \
	    smf/baseline.xml

clean:
	rm -rf $(PROTO)

clobber: clean
	rm -rf $(PKGDIR)
