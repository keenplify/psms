// Reads this Mac's Contacts for pSMS: names, company, job title, labelled numbers and emails,
// and the thumbnail photo. Prints JSON; nothing leaves the Mac from here.
ObjC.import('Contacts');
function run() {
  const store = $.CNContactStore.alloc.init;
  const keys = $([$.CNContactGivenNameKey, $.CNContactFamilyNameKey, $.CNContactOrganizationNameKey,
    $.CNContactJobTitleKey, $.CNContactPhoneNumbersKey, $.CNContactEmailAddressesKey,
    $.CNContactThumbnailImageDataKey]);
  const text = (v) => (v.isNil() ? '' : ObjC.unwrap(v));
  const labeled = (list, read) => {
    const out = [];
    for (let i = 0; i < list.count; i++) {
      const item = list.objectAtIndex(i);
      const label = item.label.isNil() ? '' : ObjC.unwrap($.CNLabeledValue.localizedStringForLabel(item.label));
      out.push({ label: label, value: read(item.value) });
    }
    return out;
  };
  const people = [];
  const request = $.CNContactFetchRequest.alloc.initWithKeysToFetch(keys);
  store.enumerateContactsWithFetchRequestErrorUsingBlock(request, null, function (c, stop) {
    const photo = c.thumbnailImageData;
    people.push({
      given: text(c.givenName), family: text(c.familyName), org: text(c.organizationName),
      title: text(c.jobTitle),
      tel: labeled(c.phoneNumbers, (v) => ObjC.unwrap(v.stringValue)),
      email: labeled(c.emailAddresses, (v) => ObjC.unwrap(v)),
      photo: photo.isNil() ? null : ObjC.unwrap(photo.base64EncodedStringWithOptions(0)),
    });
  });
  return JSON.stringify(people);
}
