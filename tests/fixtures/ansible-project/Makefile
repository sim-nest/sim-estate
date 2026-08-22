.PHONY: observe preview perform verify
observe:
	@ansible-playbook -i inventory.json playbook.yml --list-hosts
preview:
	@ansible-playbook -i inventory.json playbook.yml --check
perform:
	@ansible-playbook -i inventory.json playbook.yml
verify:
	@ansible-playbook -i inventory.json verify.yml
